"""Exercise worker leases against the same PostgreSQL schema as the API."""

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from hashlib import sha256
from pathlib import Path
from threading import Barrier
from unittest.mock import patch
from uuid import uuid4

from fastapi import HTTPException
from psycopg.types.json import Jsonb

from api import cancel
from shared import ARTIFACTS, connect, initialize
from worker import MAX_ATTEMPTS, claim, finish, renew

LEASE = 2
HEARTBEAT = 0.1
FAKE_CLI = """#!/usr/bin/env python3
import os
import sys
import time
from pathlib import Path

control = Path(__file__).parent
mode = os.environ["QBT_TEST_MODE"]
flags = ["--output", "--ledger", "--save-manifest", "--summary", "--equity"]
files = [Path(sys.argv[sys.argv.index(flag) + 1]) for flag in flags]
files[0].write_text("partial")
(control / f"{mode}.pid.tmp").write_text(str(os.getpid()))
(control / f"{mode}.pid.tmp").rename(control / f"{mode}.pid")
if mode == "held":
    while not (control / "release").exists():
        time.sleep(0.02)
if mode != "partial":
    for file in files:
        file.write_text(f"{mode} {file.name}")
"""


def wait_until(check):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("Timed out waiting for the worker")


def process_status(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError):
        return None


def process_is_running(pid):
    # An orphan can remain a zombie until the host reaps it.
    return process_status(pid) not in (None, "Z")


def waiting_on_lock(application_name, count):
    with connect() as db:
        waiting = db.execute("""
            SELECT count(*) AS count FROM pg_stat_activity
            WHERE application_name = %s AND wait_event_type = 'Lock'
        """, (application_name,)).fetchone()["count"]
    return waiting == count


class WorkerRecoveryTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        initialize()

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.work = Path(tmp.name)
        self.binary = self.work / "qbt"
        self.binary.write_text(FAKE_CLI + f"# {uuid4()}\n")
        self.binary.chmod(0o755)
        self.build = sha256(self.binary.read_bytes()).hexdigest()

        self.run_id = uuid4()
        dataset = self.run_id.hex
        with connect() as db:
            db.execute("INSERT INTO qbt_datasets VALUES (%s, %s)", (dataset, b"x\n1\n"))
            db.execute("INSERT INTO qbt_runs (id, dataset, build, options) VALUES (%s, %s, %s, %s)",
                       (self.run_id, dataset, self.build, Jsonb({})))
        self.addCleanup(self.remove_run, dataset)

    def remove_run(self, dataset):
        with connect() as db:
            db.execute("DELETE FROM qbt_artifacts WHERE run_id = %s", (self.run_id,))
            db.execute("DELETE FROM qbt_runs WHERE id = %s", (self.run_id,))
            db.execute("DELETE FROM qbt_datasets WHERE id = %s", (dataset,))

    def start_worker(self, mode, timeout=20):
        worker = subprocess.Popen([sys.executable, str(Path(__file__).with_name("worker.py"))],
                                  env={**os.environ, "QBT_BINARY": str(self.binary), "TMPDIR": str(self.work),
                                       "QBT_WORKER_LEASE_SECONDS": str(LEASE),
                                       "QBT_WORKER_HEARTBEAT_SECONDS": str(HEARTBEAT),
                                       "QBT_WORKER_TIMEOUT_SECONDS": str(timeout), "QBT_TEST_MODE": mode})
        self.addCleanup(self.stop_worker, worker)
        pidfile = self.work / f"{mode}.pid"
        wait_until(pidfile.exists)
        return worker, int(pidfile.read_text())

    @staticmethod
    def stop_worker(worker):
        if worker.poll() is None:
            worker.terminate()
        worker.wait(timeout=10)

    def run_state(self):
        with connect() as db:
            return db.execute("""
                SELECT state, error, attempt, attempts, expires_at, clock_timestamp() AS observed_at
                FROM qbt_runs WHERE id = %s
            """, (self.run_id,)).fetchone()

    def artifacts(self):
        with connect() as db:
            return {row["name"]: row["content"] for row in db.execute(
                "SELECT name, content FROM qbt_artifacts WHERE run_id = %s", (self.run_id,))}

    def expire_lease(self):
        with connect() as db:
            self.expire_lease_in(db)

    def expire_lease_in(self, db):
        db.execute("UPDATE qbt_runs SET expires_at = clock_timestamp() WHERE id = %s", (self.run_id,))

    def lock_run(self, db):
        db.execute("SELECT id FROM qbt_runs WHERE id = %s FOR UPDATE", (self.run_id,))

    def write_artifacts(self, mode):
        for name in ARTIFACTS:
            (self.work / name).write_text(f"{mode} {name}")

    def assert_artifacts(self, mode):
        self.assertEqual(self.artifacts(), {name: f"{mode} {name}".encode() for name in ARTIFACTS})

    def test_active_worker_renews_past_original_expiry(self):
        _, pid = self.start_worker("held")
        original = self.run_state()
        wait_until(lambda: self.run_state()["observed_at"] > original["expires_at"])
        current = self.run_state()
        self.assertEqual(current["attempt"], original["attempt"])
        self.assertGreater(current["expires_at"], current["observed_at"])
        self.assertIsNone(claim(self.build, LEASE))
        self.assertTrue(process_is_running(pid))
        self.assertEqual(self.artifacts(), {})
        (self.work / "release").touch()
        wait_until(lambda: self.run_state()["state"] == "succeeded")
        self.assert_artifacts("held")

    def test_worker_death_stops_cli_and_replacement_publishes(self):
        first, first_pid = self.start_worker("held")
        original = self.run_state()
        first.kill()
        first.wait(timeout=10)
        wait_until(lambda: not process_is_running(first_pid))
        self.assertEqual(self.artifacts(), {})
        self.assertEqual(self.run_state()["attempt"], original["attempt"])

        self.start_worker("replacement")
        wait_until(lambda: self.run_state()["state"] == "succeeded")
        self.assertEqual(self.run_state()["attempts"], 2)
        self.assert_artifacts("replacement")

    def test_replaced_worker_stops_its_cli_when_it_resumes(self):
        first, first_pid = self.start_worker("held")
        first.send_signal(signal.SIGSTOP)
        try:
            wait_until(lambda: process_status(first.pid) == "T")
            self.expire_lease()
            self.start_worker("replacement")
            wait_until(lambda: self.run_state()["state"] == "succeeded")
            self.assertTrue(process_is_running(first_pid))
        finally:
            first.send_signal(signal.SIGCONT)

        wait_until(lambda: not process_is_running(first_pid))
        self.assertEqual(self.run_state()["attempts"], 2)
        self.assert_artifacts("replacement")

    def test_expired_owner_cannot_renew_or_publish_around_takeover(self):
        first = claim(self.build, 20)
        self.expire_lease()
        self.assertFalse(renew(first, 20))
        self.assertFalse(finish(first, self.work))
        self.assertEqual(self.artifacts(), {})

        barrier = Barrier(2, timeout=10)

        def competing_claim():
            barrier.wait()
            return claim(self.build, 20)

        with ThreadPoolExecutor(max_workers=2) as workers:
            futures = [workers.submit(competing_claim) for _ in range(2)]
            results = [future.result() for future in futures]
        claimed = [run for run in results if run is not None]
        self.assertEqual(len(claimed), 1)
        replacement = claimed[0]
        self.assertNotEqual(first["attempt"], replacement["attempt"])
        original_expiry = self.run_state()["expires_at"]

        self.write_artifacts("stale")
        self.assertFalse(renew(first, 20))
        self.assertFalse(finish(first, self.work))
        self.assertFalse(finish(first, None, "Stale failure"))
        self.assertEqual(self.artifacts(), {})
        current = self.run_state()
        self.assertEqual(current["attempt"], replacement["attempt"])
        self.assertEqual(current["expires_at"], original_expiry)
        self.assertEqual(current["attempts"], 2)

        self.write_artifacts("replacement")
        self.assertTrue(finish(replacement, self.work))
        self.assert_artifacts("replacement")

    def test_renew_and_finish_lose_if_lease_expires_while_waiting_for_lock(self):
        attempt = claim(self.build, 20)
        self.write_artifacts("stale")
        with patch.dict(os.environ, PGAPPNAME=self.run_id.hex), ThreadPoolExecutor(max_workers=2) as workers:
            with connect() as locker:
                self.lock_run(locker)
                heartbeat = workers.submit(renew, attempt, 20)
                publish = workers.submit(finish, attempt, self.work)
                wait_until(lambda: waiting_on_lock(self.run_id.hex, 2))
                self.expire_lease_in(locker)
            self.assertFalse(heartbeat.result())
            self.assertFalse(publish.result())
        self.assertEqual(self.run_state()["state"], "running")
        self.assertEqual(self.artifacts(), {})

    def test_cancel_and_finish_serialize_without_partial_artifacts(self):
        attempt = claim(self.build, 20)
        self.write_artifacts("finished")
        with patch.dict(os.environ, PGAPPNAME=self.run_id.hex), ThreadPoolExecutor(max_workers=2) as workers:
            with connect() as locker:
                self.lock_run(locker)
                cancel_run = workers.submit(cancel, self.run_id)
                wait_until(lambda: waiting_on_lock(self.run_id.hex, 1))
                publish = workers.submit(finish, attempt, self.work)
                wait_until(lambda: waiting_on_lock(self.run_id.hex, 2))
            self.assertEqual(cancel_run.result()["state"], "cancelled")
            self.assertFalse(publish.result())
        self.assertFalse(renew(attempt, 20))
        self.assertEqual(self.run_state()["state"], "cancelled")
        self.assertEqual(self.artifacts(), {})

    def test_finish_before_cancel_retains_published_artifacts(self):
        attempt = claim(self.build, 20)
        self.write_artifacts("finished")
        with patch.dict(os.environ, PGAPPNAME=self.run_id.hex), ThreadPoolExecutor(max_workers=2) as workers:
            with connect() as locker:
                self.lock_run(locker)
                publish = workers.submit(finish, attempt, self.work)
                wait_until(lambda: waiting_on_lock(self.run_id.hex, 1))
                cancel_run = workers.submit(cancel, self.run_id)
                wait_until(lambda: waiting_on_lock(self.run_id.hex, 2))
            self.assertTrue(publish.result())
            with self.assertRaises(HTTPException) as raised:
                cancel_run.result()
            self.assertEqual(raised.exception.status_code, 409)
        self.assertEqual(self.run_state()["state"], "succeeded")
        self.assert_artifacts("finished")

    def test_exhausted_crashes_fail_without_another_attempt(self):
        for _ in range(MAX_ATTEMPTS):
            self.assertIsNotNone(claim(self.build, 20))
            self.expire_lease()
        self.assertIsNone(claim(self.build, 20))
        failed = self.run_state()
        self.assertEqual(failed["state"], "failed")
        self.assertEqual(failed["error"], "Worker retry limit reached")
        self.assertEqual(failed["attempts"], MAX_ATTEMPTS)
        self.assertIsNone(failed["attempt"])
        self.assertIsNone(failed["expires_at"])
        self.assertEqual(self.artifacts(), {})

    def test_incomplete_success_rolls_back_the_partial_artifact(self):
        worker, pid = self.start_worker("partial")
        wait_until(lambda: self.run_state()["state"] == "failed")
        self.assertIn("orders.csv", self.run_state()["error"])
        self.assertEqual(self.artifacts(), {})
        self.assertFalse(process_is_running(pid))
        self.assertIsNone(worker.poll())

    def test_timeout_kills_cli_and_records_failure(self):
        worker, pid = self.start_worker("held", timeout=1)
        wait_until(lambda: self.run_state()["state"] == "failed")
        self.assertIn("timed out", self.run_state()["error"])
        self.assertEqual(self.artifacts(), {})
        self.assertFalse(process_is_running(pid))
        self.assertIsNone(worker.poll())


if __name__ == "__main__":
    unittest.main()

import logging
import math
import os
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from uuid import uuid4

from psycopg import OperationalError

from shared import ARTIFACTS, BINARY, build_id, connect, initialize

MAX_ATTEMPTS = 3
IDLE_SECONDS = 0.5
LOGGER = logging.getLogger(__name__)


@dataclass(frozen=True)
class Settings:
    lease: float = 30
    heartbeat: float = 1
    timeout: float = 300

    @classmethod
    def from_environment(cls):
        values = {}
        for field, key, default in (
            ("lease", "QBT_WORKER_LEASE_SECONDS", cls.lease),
            ("heartbeat", "QBT_WORKER_HEARTBEAT_SECONDS", cls.heartbeat),
            ("timeout", "QBT_WORKER_TIMEOUT_SECONDS", cls.timeout),
        ):
            try:
                value = float(os.environ.get(key, default))
                if not math.isfinite(value) or value <= 0:
                    raise ValueError()
            except ValueError:
                raise ValueError(f"{key} must be a finite positive number") from None
            values[field] = value
        if values["heartbeat"] >= values["lease"]:
            raise ValueError("QBT_WORKER_HEARTBEAT_SECONDS must be shorter than QBT_WORKER_LEASE_SECONDS")
        return cls(**values)


def claim(build, lease):
    with connect(connect_timeout=5) as db:
        db.execute("""
            WITH exhausted AS (
                SELECT id FROM qbt_runs
                WHERE state = 'running' AND expires_at <= clock_timestamp() AND attempts >= %s
                FOR UPDATE SKIP LOCKED
            )
            UPDATE qbt_runs r SET state = 'failed', error = 'Worker retry limit reached',
                attempt = NULL, expires_at = NULL
            FROM exhausted WHERE r.id = exhausted.id
        """, (MAX_ATTEMPTS,))
        return db.execute("""
            WITH candidate AS (
                SELECT id FROM qbt_runs
                WHERE build = %s AND attempts < %s
                    AND (state = 'queued' OR (state = 'running' AND expires_at <= clock_timestamp()))
                ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED
            )
            UPDATE qbt_runs r SET state = 'running', attempts = attempts + 1, attempt = %s,
                expires_at = clock_timestamp() + %s * interval '1 second'
            FROM candidate WHERE r.id = candidate.id
            RETURNING r.id, r.dataset, r.options, r.attempt
        """, (build, MAX_ATTEMPTS, uuid4(), lease)).fetchone()


def lock_attempt(db, run):
    # Check the expiry after taking the lock, so time spent waiting for a row
    # cannot let an expired owner renew or finish.
    return db.execute("""
        SELECT id FROM qbt_runs WHERE id = %s AND attempt = %s AND state = 'running'
        FOR UPDATE
    """, (run["id"], run["attempt"])).fetchone() is not None


def renew(run, lease):
    with connect(connect_timeout=5) as db:
        if not lock_attempt(db, run):
            return False
        return db.execute("""
            UPDATE qbt_runs SET expires_at = clock_timestamp() + %s * interval '1 second'
            WHERE id = %s AND expires_at > clock_timestamp()
        """, (lease, run["id"])).rowcount == 1


def simulate(run, work, settings):
    with connect(connect_timeout=5) as db:
        content = db.execute("SELECT csv FROM qbt_datasets WHERE id = %s", (run["dataset"],)).fetchone()["csv"]
    source = work / "input.csv"
    source.write_bytes(content)
    cmd = [str(BINARY), "run", "--input", str(source), "--output", str(work / "report.html"),
           "--ledger", str(work / "orders.csv"), "--save-manifest", str(work / "manifest.json"),
           "--summary", str(work / "summary.json"), "--equity", str(work / "equity.json")]
    for name, value in run["options"].items():
        flag = "--" + name.replace("_", "-")
        if isinstance(value, bool):
            if value:
                cmd.append(flag)
        elif value is not None:
            cmd.extend((flag, str(value)))
    if not renew(run, settings.lease):
        return False

    launch = cmd
    if sys.platform == "linux":
        launcher = Path(__file__).with_name("run_cli.py")
        launch = [sys.executable, str(launcher), str(os.getpid()), *cmd]

    deadline = time.monotonic() + settings.timeout
    with subprocess.Popen(launch, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE) as proc:
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(cmd, settings.timeout)
                try:
                    _, stderr = proc.communicate(timeout=min(remaining, settings.heartbeat))
                    break
                except subprocess.TimeoutExpired:
                    if time.monotonic() >= deadline:
                        raise subprocess.TimeoutExpired(cmd, settings.timeout) from None
                    if not renew(run, settings.lease):
                        return False
            if proc.returncode:
                raise ValueError(stderr.decode(errors="replace")[-2000:] or f"QBT exited {proc.returncode}")
            return True
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.communicate()


def finish(run, work, error=None):
    with connect(connect_timeout=5) as db:
        if not lock_attempt(db, run):
            return False
        current = db.execute("""
            UPDATE qbt_runs SET state = %s, error = %s, attempt = NULL, expires_at = NULL
            WHERE id = %s AND expires_at > clock_timestamp() RETURNING id
        """, ("failed" if error is not None else "succeeded", error, run["id"])).fetchone()
        if current is None:
            return False
        if error is None:
            for name in ARTIFACTS:
                db.execute("INSERT INTO qbt_artifacts VALUES (%s, %s, %s)",
                           (run["id"], name, (work / name).read_bytes()))
        return True


def process(run, settings):
    try:
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            if simulate(run, work, settings):
                finish(run, work)
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        finish(run, None, str(exc))


def stop_worker(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    try:
        settings = Settings.from_environment()
    except ValueError as exc:
        raise SystemExit(str(exc)) from None
    if os.name == "posix":
        signal.signal(signal.SIGTERM, stop_worker)
    initialize()
    build = build_id()
    while True:
        try:
            run = claim(build, settings.lease)
            if run is not None:
                process(run, settings)
                continue
        except OperationalError as exc:
            # The lease expires if a database outage interrupts the attempt.
            LOGGER.warning("Worker database operation failed: %s", exc)
        time.sleep(IDLE_SECONDS)


if __name__ == "__main__":
    main()

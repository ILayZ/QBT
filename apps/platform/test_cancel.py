"""Integration coverage for cancellation, using an isolated build so claiming is deterministic."""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from uuid import uuid4

from playwright.sync_api import expect, sync_playwright
from psycopg.types.json import Jsonb

from shared import connect

BASE = "http://127.0.0.1:8765"


def enqueue(build, dataset):
    run_id = uuid4()
    with connect() as db:
        db.execute("INSERT INTO qbt_runs (id, dataset, build, options) VALUES (%s, %s, %s, %s)",
                   (run_id, dataset, build, Jsonb({"strategy": "DCA", "capital": 100})))
    return f"{BASE}/v1/runs/{run_id}"


def wait_until(predicate):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError("Timed out waiting for worker")


with tempfile.TemporaryDirectory() as tmp, sync_playwright() as playwright:
    binary = Path(tmp) / "qbt-test"
    pidfile = Path(tmp) / "child.pid"
    binary.write_text(f"""#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
Path(sys.argv[sys.argv.index('--output') + 1]).write_text('partial')
Path({str(pidfile)!r}).write_text(str(os.getpid()))
time.sleep(120)
""")
    binary.chmod(0o755)
    build = hashlib.sha256(binary.read_bytes()).hexdigest()
    browser = playwright.chromium.launch(executable_path=shutil.which("google-chrome") or shutil.which("chromium"))
    page = browser.new_page()
    http = page.request
    dataset = http.post(f"{BASE}/v1/datasets", headers={"Content-Type": "text/csv"},
                        data=Path("data/sample_btc_1h.csv").read_bytes()).json()["id"]
    queued = enqueue(build, dataset)
    page.goto(BASE + "/?run=" + queued.rsplit("/", 1)[1])
    page.get_by_role("button", name="Cancel run").click()
    expect(page.get_by_role("status")).to_have_text("cancelled")
    assert http.post(queued + "/cancel").ok
    assert http.get(queued).json()["artifacts"] == {}

    running = enqueue(build, dataset)
    worker = subprocess.Popen([sys.executable, str(Path(__file__).with_name("worker.py"))],
                              env={**os.environ, "QBT_BINARY": str(binary)})
    try:
        wait_until(lambda: pidfile.exists() and pidfile.stat().st_size > 0)
        pid = int(pidfile.read_text())
        assert http.get(running).json()["state"] == "running"
        assert http.post(running + "/cancel").ok
        wait_until(lambda: not Path(f"/proc/{pid}").exists())
        assert http.get(running).json()["state"] == "cancelled"
        assert http.get(running + "/artifacts/report.html").status == 404
        assert worker.poll() is None
        with connect() as db:
            completed = db.execute("SELECT id FROM qbt_runs WHERE state = 'succeeded' LIMIT 1").fetchone()["id"]
        assert http.post(f"{BASE}/v1/runs/{completed}/cancel").status == 409
        assert http.get(f"{BASE}/v1/runs/{completed}/artifacts/report.html").ok
    finally:
        worker.terminate()
        worker.wait(timeout=10)
        browser.close()

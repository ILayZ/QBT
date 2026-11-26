import subprocess
import tempfile
import time
from pathlib import Path
from uuid import uuid4

from shared import ARTIFACTS, BINARY, build_id, connect, initialize


def claim(build):
    token = uuid4()
    with connect() as db:
        db.execute("""
            UPDATE qbt_runs SET state = 'failed', error = 'Worker retry limit reached', attempt = NULL
            WHERE state = 'running' AND expires_at < now() AND attempts >= 3
        """)
        return db.execute("""
            WITH candidate AS (
                SELECT id FROM qbt_runs
                WHERE build = %s AND (state = 'queued' OR (state = 'running' AND expires_at < now()))
                ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED
            )
            UPDATE qbt_runs r SET state = 'running', attempts = attempts + 1, attempt = %s,
                expires_at = now() + interval '6 minutes'
            FROM candidate WHERE r.id = candidate.id
            RETURNING r.id, r.dataset, r.options, r.attempt
        """, (build, token)).fetchone()


def simulate(run, work):
    with connect() as db:
        dataset = db.execute("""
            SELECT csv, format, symbol, quote_currency FROM qbt_datasets WHERE id = %s
        """, (run["dataset"],)).fetchone()
    source = work / f"input.{dataset['format']}"
    source.write_bytes(dataset["csv"])
    cmd = [str(BINARY), "run", "--input", str(source), "--output", str(work / "report.html"),
           "--ledger", str(work / "orders.csv"), "--save-manifest", str(work / "manifest.json"),
           "--summary", str(work / "summary.json"), "--equity", str(work / "equity.json")]
    options = {**run["options"], "symbol": dataset["symbol"], "quote_currency": dataset["quote_currency"]}
    for name, value in options.items():
        flag = "--" + name.replace("_", "-")
        if isinstance(value, bool):
            if value:
                cmd.append(flag)
        elif value is not None:
            cmd.extend((flag, str(value)))
    deadline = time.monotonic() + 300
    with subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE) as proc:
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(cmd, 300)
                try:
                    _, stderr = proc.communicate(timeout=min(remaining, 1))
                    break
                except subprocess.TimeoutExpired:
                    with connect() as db:
                        owned = db.execute("""
                            SELECT 1 FROM qbt_runs WHERE id = %s AND attempt = %s AND state = 'running'
                        """, (run["id"], run["attempt"])).fetchone()
                    if owned is None:
                        # Cancellation or lease takeover revoked publication rights.
                        return
            if proc.returncode:
                raise ValueError(stderr.decode(errors="replace")[-2000:] or f"QBT exited {proc.returncode}")
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.communicate()


def finish(run, work, error=None):
    with connect() as db:
        # The attempt owning the row publishes the entire set in one transaction.
        current = db.execute("""
            UPDATE qbt_runs SET state = %s, error = %s, attempt = NULL, expires_at = NULL
            WHERE id = %s AND attempt = %s AND state = 'running' RETURNING id
        """, ("failed" if error else "succeeded", error, run["id"], run["attempt"])).fetchone()
        if current and error is None:
            for name in ARTIFACTS:
                db.execute("INSERT INTO qbt_artifacts VALUES (%s, %s, %s)",
                           (run["id"], name, (work / name).read_bytes()))


def main():
    initialize()
    build = build_id()
    while True:
        run = claim(build)
        if run is None:
            time.sleep(0.5)
            continue
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            try:
                simulate(run, work)
            except (ValueError, subprocess.TimeoutExpired) as exc:
                finish(run, work, str(exc))
            else:
                finish(run, work)


if __name__ == "__main__":
    main()

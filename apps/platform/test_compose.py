"""Smoke-test a running Compose project and restore its backup to a new database."""

import hashlib
import json
import os
import subprocess
import time
from pathlib import Path
from urllib.request import Request, urlopen
from uuid import UUID, uuid4

BASE = f"http://127.0.0.1:{os.environ.get('QBT_PORT', '8765')}"
EXPECTED_ARTIFACTS = {"report.html", "orders.csv", "summary.json", "equity.json", "manifest.json"}


def request(path, data=None, content_type="application/json"):
    with urlopen(Request(BASE + path, data=data, headers={"Content-Type": content_type}), timeout=10) as response:
        return response.read()


def docker(*args, data=None):
    return subprocess.run(["docker", *args], input=data, stdout=subprocess.PIPE, check=True).stdout


def postgres(*args, data=None):
    return docker("compose", "exec", "-T", "postgres", *args, data=data)


def database_result(db, run_id):
    return json.loads(postgres(
        "psql", "-XAt", "-v", "ON_ERROR_STOP=1", "-U", "qbt", "-d", db, "-c",
        f"""
        SELECT json_build_object(
            'dataset', d.id,
            'format', d.format,
            'symbol', d.symbol,
            'quote_currency', d.quote_currency,
            'validation', d.validation,
            'input_sha256', encode(sha256(d.csv), 'hex'),
            'state', r.state,
            'artifacts', (
                SELECT json_object_agg(name, encode(sha256(content), 'hex'))
                FROM qbt_artifacts WHERE run_id = r.id
            )
        )
        FROM qbt_runs r JOIN qbt_datasets d ON d.id = r.dataset
        WHERE r.id = '{run_id}'
        """,
    ))


def main():
    assert json.loads(request("/healthz")) == {"status": "ok"}
    ids = [docker("compose", "ps", "-q", service).decode().strip() for service in ("api", "worker")]
    images = [docker("inspect", "--format", "{{.Image}}", cid) for cid in ids]
    assert images[0] == images[1], images
    cli_hash = docker("compose", "exec", "-T", "worker", "sha256sum", "/usr/local/bin/qbt").decode().split()[0]
    port_bindings = json.loads(docker("inspect", "--format", "{{json .HostConfig.PortBindings}}", ids[0]))
    assert port_bindings == {"8765/tcp": [{"HostIp": "127.0.0.1", "HostPort": BASE.rsplit(":", 1)[1]}]}, port_bindings

    source = Path("data/sample_btc_1h.csv").read_bytes()
    metadata = json.loads(request("/v1/datasets?symbol=BTC&quote_currency=USD", source, "text/csv"))
    dataset = metadata["id"]
    assert dataset == hashlib.sha256(source).hexdigest()
    assert metadata["row_count"] == 4320 and metadata["schema_version"] == 1
    assert json.loads(request(f"/v1/datasets/{dataset}")) == metadata
    payload = json.dumps({"dataset": dataset, "options": {"strategy": "all", "capital": 1800}}).encode()
    job = json.loads(request("/v1/runs", payload))
    run_id = UUID(job["id"])
    assert job["state"] == "queued", job
    for _ in range(120):
        status = json.loads(request(f"/v1/runs/{run_id}"))
        if status["state"] in ("succeeded", "failed"):
            break
        time.sleep(0.5)
    assert status["state"] == "succeeded", status
    assert status["build"] == cli_hash
    assert status["artifacts"].keys() == EXPECTED_ARTIFACTS, status
    artifacts = {name: request(path) for name, path in status["artifacts"].items()}
    assert b"<svg" in artifacts["report.html"]
    assert b"strategy" in artifacts["orders.csv"].splitlines()[0]
    for name in ("summary.json", "equity.json"):
        assert json.loads(artifacts[name])["version"] == 1
    assert json.loads(artifacts["manifest.json"])["spec"]["include_benchmarks"]

    assert json.loads(artifacts["summary.json"])["quote_currency"] == "USD"
    expected = {
        "dataset": dataset,
        "format": "csv",
        "symbol": "BTC",
        "quote_currency": "USD",
        "validation": {key: metadata[key] for key in ("schema_version", "timeframe", "row_count", "start", "end")},
        "input_sha256": dataset,
        "state": "succeeded",
        "artifacts": {name: hashlib.sha256(data).hexdigest() for name, data in artifacts.items()},
    }
    assert database_result("qbt", run_id) == expected
    backup = postgres("pg_dump", "-U", "qbt", "-d", "qbt", "--format=custom")
    restored = f"qbt_restore_{uuid4().hex}"
    postgres("createdb", "-U", "qbt", "--template=template0", restored)
    postgres("pg_restore", "-U", "qbt", "-d", restored, "--no-owner", "--no-privileges",
             "--single-transaction", data=backup)
    assert database_result(restored, run_id) == expected
    assert database_result("qbt", run_id) == expected
    print(f"Verified the shared CLI, run {run_id}, and byte-identical dataset and artifacts in {restored}")


if __name__ == "__main__":
    main()

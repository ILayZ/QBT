"""End-to-end check against a running API and worker using the release binary."""
import csv
import json
import os
import subprocess
import tempfile
from datetime import datetime
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

BASE = "http://127.0.0.1:8765"


def request(path, data=None, content_type="application/json"):
    with urlopen(Request(BASE + path, data=data, headers={"Content-Type": content_type}), timeout=10) as response:
        return response.read()


def run(csv, expected, options):
    dataset = json.loads(request("/v1/datasets", csv, "text/csv"))["id"]
    payload = json.dumps({"dataset": dataset, "options": options}).encode()
    job = json.loads(request("/v1/runs", payload))
    url = "/v1/runs/" + job["id"]
    assert job["state"] == "queued"
    for _ in range(120):
        status = json.loads(request(url))
        if status["state"] in ("failed", "succeeded"):
            break
        time.sleep(0.5)
    assert status["state"] == expected, status
    return status


if __name__ == "__main__":
    for attempt in range(40):
        try:
            request("/openapi.json")
            break
        except OSError:
            if attempt == 39:
                raise
            time.sleep(0.5)
    options = {"strategy": "all", "capital": 1800, "daily_limit": 10, "order_amount": 10, "min_notional": 1}
    status = run(Path("data/sample_btc_1h.csv").read_bytes(), "succeeded", options)
    files = status["artifacts"]
    assert request(files["orders.csv"]) == Path(".e2e/min.csv").read_bytes()
    assert request(files["report.html"]) == Path(".e2e/min.html").read_bytes()
    assert json.loads(request(files["manifest.json"]))["spec"]["include_benchmarks"]
    summary = json.loads(request(files["summary.json"]))
    equity = json.loads(request(files["equity.json"]))
    assert summary["version"] == equity["version"] == 1
    portfolios = {r["name"]: r for r in summary["portfolios"]}
    assert set(portfolios) == {"DCA", "DipDCA", "VolAdjustedDCA", "MomentumSMA", "RegimeSwitch", "Cash", "Buy and hold"}
    assert round(portfolios["DCA"]["final_equity"], 2) == 1875.49
    dip = portfolios["DipDCA"]
    assert round(dip["final_equity"], 2) == 1862.67
    assert round(dip["difference_vs_dca"], 2) == -12.82
    assert round(dip["unspent_reserve"], 2) == 879.08 and dip["max_reserve_wait_hours"] == 386
    source = Path("data/sample_btc_1h.csv")
    with source.open() as stream:
        times = [datetime.fromisoformat(row["timestamp"]) for row in csv.DictReader(stream)]
    assert [datetime.fromisoformat(t) for t in equity["timestamps"]] == times
    assert {p["name"] for p in equity["portfolios"]} == set(portfolios)
    for curve in equity["portfolios"]:
        assert len(curve["values"]) == len(times)
        assert curve["values"][0] == 1800
        assert curve["values"][-1] == portfolios[curve["name"]]["final_equity"]
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        (tmp / "manifest.json").write_bytes(request(files["manifest.json"]))
        subprocess.run([os.environ.get("QBT_BINARY", "target/release/qbt"), "replay",
                        "--manifest", str(tmp / "manifest.json"), "--input", str(source),
                        "--output", str(tmp / "report.html"), "--summary", str(tmp / "summary.json"),
                        "--equity", str(tmp / "equity.json")], check=True)
        for name in ("summary.json", "equity.json"):
            assert request(files[name]) == (tmp / name).read_bytes()
    failed = run(b"not,a,dataset\n1,2,3\n", "failed", {})
    assert failed["error"] and not failed["artifacts"]
    try:
        request(f"/v1/runs/{failed['id']}/artifacts/report.html")
    except HTTPError as exc:
        assert exc.code == 404
    else:
        raise AssertionError("Published a failed run")

"""End-to-end check against a running API and worker using the release binary."""
import json
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
    failed = run(b"not,a,dataset\n1,2,3\n", "failed", {})
    assert failed["error"] and not failed["artifacts"]
    try:
        request(f"/v1/runs/{failed['id']}/artifacts/report.html")
    except HTTPError as exc:
        assert exc.code == 404
    else:
        raise AssertionError("Published a failed run")

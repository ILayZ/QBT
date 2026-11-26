# Local strategy testing service

Run the single-user API and worker from the repository root with Python 3.11+, PostgreSQL 16 and the release CLI. Use a dedicated empty database. Both processes initialize the schema on startup. Keep the API bound to loopback.

```bash
cargo build --release --locked
python3 -m venv .venv
source .venv/bin/activate
pip install -r apps/platform/requirements.txt
export DATABASE_URL=postgresql://localhost/qbt
python apps/platform/worker.py &
python -m uvicorn api:app --app-dir apps/platform --host 127.0.0.1 --port 8765
```

Open http://127.0.0.1:8765 in a browser to upload a CSV, choose a strategy or compare all five, and download results. Open a previous result from Run history or bookmark its URL. History includes runs submitted directly through the API. Use `GET /v1/runs?limit=20` to list recent runs and pass the returned `next` ID as `before` for the following page. The form loads strategy names and defaults from the API.

In a second shell, upload the sample and submit a comparison. These examples use jq.

```bash
base=http://127.0.0.1:8765
dataset=$(curl -fsS -H 'Content-Type: text/csv' --data-binary @data/sample_btc_1h.csv "$base/v1/datasets" | jq -r .id)
run=$(curl -fsS -H 'Content-Type: application/json' -d "$(jq -nc --arg id "$dataset" '{dataset:$id,options:{strategy:"all",capital:1800,order_amount:10,daily_limit:10}}')" "$base/v1/runs" | jq -r .id)
curl -fsS "$base/v1/runs/$run" | jq
# After state becomes succeeded:
curl -fsS "$base/v1/runs/$run/artifacts/report.html" -o report.html
curl -fsS "$base/v1/runs/$run/artifacts/orders.csv" -o orders.csv
curl -fsS "$base/v1/runs/$run/artifacts/manifest.json" -o manifest.json
```

Interactive endpoint documentation is at `/docs`. Upload accepts raw CSV up to 8 MiB. Identical bytes share one dataset ID. Upload records the bytes, and the engine validates the bars during a run. A failed simulation exposes its error through the status endpoint.

This first deployment stores inputs, job state and artifacts together in PostgreSQL. Workers claim jobs for their binary's SHA-256. Keep the binary immutable while either process runs and restart both when upgrading. An expired six-minute lease allows another worker of the same build to recover a job. A five-minute subprocess timeout bounds execution. The attempt token fences publication, and all three artifacts become visible in one transaction. The API supports single and comparison runs. Project authentication, walk-forward submission and external object storage are planned.

Use **Cancel run** in the workspace or `POST /v1/runs/{id}/cancel` to cancel a queued or running job. A successful cancellation immediately revokes the attempt and prevents artifact publication. The worker checks ownership at one-second intervals during execution. Repeating a cancellation succeeds. If completion won the race, the API returns HTTP 409 and preserves the result.

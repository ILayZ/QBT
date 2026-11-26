# Local strategy testing service

Run the single-user API and worker from the repository root with Python 3.11+, PostgreSQL 16 and the release CLI. Use a dedicated database. Both processes initialize or upgrade the schema on startup, preserving existing runs. Keep the API bound to loopback.

```bash
cargo build --release --locked
python3 -m venv .venv
source .venv/bin/activate
pip install -r apps/platform/requirements.txt
export DATABASE_URL=postgresql://localhost/qbt
python apps/platform/worker.py &
python -m uvicorn api:app --app-dir apps/platform --host 127.0.0.1 --port 8765
```

Open http://127.0.0.1:8765 in a browser to upload CSV or Parquet data or select a saved dataset, choose a strategy or compare all five, and download results. Open a previous result from Run history or bookmark its URL. History includes runs submitted directly through the API. Use `GET /v1/runs?limit=20` to list recent runs and pass the returned `next` ID as `before` for the following page. The form loads strategy names and defaults from the API.

In a second shell, upload the sample and submit a comparison. These examples use jq.

```bash
base=http://127.0.0.1:8765
dataset=$(curl -fsS -H 'Content-Type: text/csv' --data-binary @data/sample_btc_1h.csv "$base/v1/datasets?symbol=BTC&quote_currency=USD" | jq -r .id)
run=$(curl -fsS -H 'Content-Type: application/json' -d "$(jq -nc --arg id "$dataset" '{dataset:$id,options:{strategy:"all",capital:1800,order_amount:10,daily_limit:10}}')" "$base/v1/runs" | jq -r .id)
curl -fsS "$base/v1/runs/$run" | jq
# After state becomes succeeded:
curl -fsS "$base/v1/runs/$run/artifacts/report.html" -o report.html
curl -fsS "$base/v1/runs/$run/artifacts/orders.csv" -o orders.csv
curl -fsS "$base/v1/runs/$run/artifacts/manifest.json" -o manifest.json
```

Interactive endpoint documentation is at `/docs`. Send raw data up to 8 MiB to `POST /v1/datasets` with `Content-Type: text/csv` or `application/vnd.apache.parquet`. QBT validates the bars before saving them. Invalid data returns HTTP 422 with a diagnostic. Failed simulations expose their errors through the run status endpoint.

The catalog uses the SHA-256 of the uploaded bytes as the dataset ID. Uploading the same bytes returns the existing ID. Changing the bytes creates a new entry. Optional `symbol` and `quote_currency` query parameters can fill missing metadata. Changing a recorded value for the same ID returns HTTP 409. Values must start with an ASCII letter or digit and may include periods, underscores, colons, slashes and hyphens, up to 64 characters.

Use `GET /v1/datasets` to list saved datasets and `GET /v1/datasets/{id}` for one entry. The response includes format, symbol, quote currency, validation schema version, timeframe, row count and inclusive UTC start and end. Datasets saved before validation have null validation fields and their IDs remain usable. Reuploading their original bytes records validation metadata if the bars pass.

To retry a submission safely, send an optional `Idempotency-Key` header with `POST /v1/runs`. Use one case-sensitive key of 1 to 128 visible ASCII characters without commas, such as a UUID. The key applies across the local service. A retry with the same dataset ID and validated options, including defaults, returns HTTP 202 with the original run ID and its current state, even after completion. Reusing a key with a different dataset or options returns HTTP 409. Each request without the header creates a new run. Keys are retained with their runs.

This first deployment stores inputs, job state and artifacts together in PostgreSQL. Workers claim jobs for their binary's SHA-256. Keep the binary immutable while either process runs and restart both when upgrading. An expired six-minute lease allows another worker of the same build to recover a job. A five-minute subprocess timeout bounds execution. The attempt token fences publication, and all artifacts become visible in one transaction. The API supports single and comparison runs. Project authentication, walk-forward submission and external object storage are planned.

Use **Cancel run** in the workspace or `POST /v1/runs/{id}/cancel` to cancel a queued or running job. A successful cancellation immediately revokes the attempt and prevents artifact publication. The worker checks ownership at one-second intervals during execution. Repeating a cancellation succeeds. If completion won the race, the API returns HTTP 409 and preserves the result.

### Machine-readable results

New single and comparison runs publish `summary.json` and `equity.json` alongside the HTML, ledger and manifest. Discover available artifacts through `GET /v1/runs/{id}`. Earlier runs retain their original downloads. For a local run or replay, pass `--summary summary.json --equity equity.json`.

Both JSON documents carry `version: 1`. The summary includes `symbol` and `quote_currency` when supplied with the dataset. The replay manifest records these values so replay reproduces the summary. Summary returns, drawdowns and weights are fractions. Equity, DCA differences and reserves use the input's quote currency. Completed trades count closed trades, while the ledger includes buys and skipped orders. DCA differences are null when DCA is absent. Reserve fields apply to DipDCA. Equity contains one ordered UTC bar timestamp array and one equally long array of close valuations per portfolio, including benchmarks. Values retain engine precision.

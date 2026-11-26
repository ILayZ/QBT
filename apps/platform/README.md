# Local strategy testing service

Run the API, one worker, and PostgreSQL 16 on the same trusted host. With Docker Engine 28 or later (or a current Docker Desktop) and the Docker Compose v2 plugin, run this from the repository root.

```bash
docker compose up --build --wait
```

The first build compiles the Rust release CLI and may take several minutes. The API and worker share the resulting image and use the same read-only CLI. Compose waits for PostgreSQL to accept connections and for the API to finish initializing the schema and answer `GET /healthz`. The worker starts after the API becomes healthy. Use `docker compose ps` and `docker compose logs -f api worker postgres` to check the services.

HTTP is published on `127.0.0.1:8765`. Set `QBT_PORT` before starting the stack to use another host port. PostgreSQL has an internal network and no published port. The workspace assumes a single trusted user. The default database password is for local development. To set your own, set `QBT_DB_PASSWORD` to a URL-safe value before the volume is created and keep using that setting on subsequent starts. Changing the setting alone does not alter an existing PostgreSQL role.

The worker executes one simulation at a time with two CPUs, 1 GiB of memory, and 128 MiB of temporary storage. A simulation has a five-minute execution timeout. The API runs one process on one CPU with a 512 MiB memory limit and a Uvicorn concurrency limit of 32. PostgreSQL has one CPU, 512 MiB of memory, and at most 64 connections. Dataset, run, and artifact tables share the `qbt_postgres-data` volume. Plan disk space for the build cache, database, and backups.

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

This first deployment stores inputs, job state and artifacts together in PostgreSQL. Workers claim jobs for their binary's SHA-256. Keep the binary immutable while either process runs and restart both when upgrading. An expired six-minute lease allows a worker of the same build to recover a job. The attempt token fences publication, and all artifacts become visible in one transaction. The API supports single and comparison runs. Project authentication, walk-forward submission and external object storage are planned.

Use **Cancel run** in the workspace or `POST /v1/runs/{id}/cancel` to cancel a queued or running job. A successful cancellation immediately revokes the attempt and prevents artifact publication. The worker checks ownership at one-second intervals during execution. Repeating a cancellation succeeds. If completion won the race, the API returns HTTP 409 and preserves the result.

## Stop, back up, and restore

`docker compose down` removes the containers and retains the PostgreSQL volume. Run the startup command again to resume. Keep the same Compose project name to reuse the volume.

To back up the live database, write a PostgreSQL custom-format dump to a new, private file. Keep a copy on a separate drive or host.

```bash
backup=$(mktemp qbt-backup.XXXXXXXX.dump)
if docker compose exec -T postgres pg_dump -U qbt -d qbt --format=custom > "$backup"; then
  printf '%s\n' "$backup"
else
  rm -- "$backup"
  exit 1
fi
```

Restore into a fresh database to validate the dump. Replace the example archive name and choose an unused database name. `createdb` must succeed before `pg_restore` runs.

```bash
backup=qbt-backup.ABCDEFGH.dump
restore_db=qbt_restore_check
docker compose exec -T postgres createdb -U qbt --template=template0 "$restore_db" &&
docker compose exec -T postgres pg_restore -U qbt -d "$restore_db" \
  --no-owner --no-privileges --single-transaction < "$backup" &&
docker compose exec -T postgres psql -X -v ON_ERROR_STOP=1 -U qbt -d "$restore_db" \
  -c 'SELECT count(*) FROM qbt_datasets;' \
  -c 'SELECT state, count(*) FROM qbt_runs GROUP BY state;' \
  -c 'SELECT count(*) FROM qbt_artifacts;'
```

Compare the restored counts against the expected snapshot before relying on a backup. The Compose workflow checks dataset and artifact hashes after restoring to a fresh database. To recover a deployment, restore into a fresh database as above, stop the API and worker, and set `DATABASE_URL` for both services in a Compose override to use the restored database. Keep the original database available until you verify the recovered deployment.

## Upgrade

Before upgrading QBT, stop the API with `docker compose stop api` to close submissions. Let the old worker finish queued and running jobs. Check for those jobs with:

```bash
docker compose exec -T postgres psql -U qbt -d qbt \
  -c "SELECT state, count(*) FROM qbt_runs WHERE state IN ('queued', 'running') GROUP BY state;"
```

Then stop the worker with `docker compose stop worker`, take a backup, and run the startup command from the new checkout under the same project name. Workers claim jobs for their exact CLI build, so incomplete jobs may need the previous image to finish. Both services initialize the existing schema transactionally under a PostgreSQL advisory lock. For a release with schema changes, rehearse its upgrade on a restored copy and follow that release's migration instructions. Changing the PostgreSQL major version requires a separate database upgrade or dump and restore.

## Machine-readable results

New single and comparison runs publish `summary.json` and `equity.json` alongside the HTML, ledger and manifest. Discover available artifacts through `GET /v1/runs/{id}`. Earlier runs retain their original downloads. For a local run or replay, pass `--summary summary.json --equity equity.json`.

Both JSON documents carry `version: 1`. Summary returns, drawdowns and weights are fractions. Equity, DCA differences and reserves use the input's quote currency. Completed trades count closed trades, while the ledger includes buys and skipped orders. DCA differences are null when DCA is absent. Reserve fields apply to DipDCA. Equity contains one ordered UTC bar timestamp array and one equally long array of close valuations per portfolio, including benchmarks. Values retain engine precision.

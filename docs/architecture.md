# QBT platform architecture

Status: proposed. The repository currently ships the Rust engine, local CLI and HTML renderer. The platform described below is a design for the next releases.

## Deployment and ownership

Start with a modular API and a separately runnable worker in the same repository. A single machine can host both. Increase the worker count when simulations contend with interactive API requests.

| Component | Owns | Boundary |
| --- | --- | --- |
| Web application | Dataset upload, strategy and budget selection, progress, comparison and drill-down into skipped orders | Calls the API with project-scoped credentials |
| API | Projects, access checks, dataset catalog, strategy catalog, validation, run submission and cancellation | Returns a run ID immediately after committing the job |
| Worker | Dataset validation and normalization, dataset loading, engine invocation and artifact publication | Claims jobs from PostgreSQL and links the compiled QBT library |
| QBT Rust library | Strategy decisions, simulated execution, cash and positions, benchmarks, window evaluation, metrics | Takes a resolved specification and a validated price series. Returns typed results |
| PostgreSQL | Project membership, dataset versions, run specifications, states, leases, summary metrics and artifact references | Transactional source of truth for visibility and job ownership |
| Object storage | Immutable input snapshots, order ledger, equity curves and optional HTML exports | Private objects accessed through authorized, short-lived URLs |

The local CLI reads local files and calls the same library directly. Keep database, HTTP and job types outside the simulation modules. The Cargo workspace contains the library in `crates/qbt-engine` and the `qbt` binary in `apps/cli`. Add the API, worker and web application under `apps/` as they are implemented. Each deployable has its own build target.

## Contract between platform and engine

Introduce a versioned `RunSpec` and a library entry point shared by the CLI and worker. Extract strategy construction and comparison orchestration from `apps/cli/src/cli/mod.rs`. Within `crates/qbt-engine`, keep fills and accounting in `src/backtest`, signals in `src/strategy`, metrics in `src/metrics` and HTML generation in `src/report`. A registry supplies strategy IDs, parameter types, ranges and defaults to both the CLI and the API.

Persist a fully resolved spec when accepting a run. Include schema version, engine build/commit, dataset version and normalized content hash, symbol, timeframe, UTC range, mode, strategy IDs and parameters, initial capital, order sizing, fees, slippage, risk-free rate, minimum notional and daily budget including its fee policy. Include benchmark and window settings when applicable. A worker must claim only builds it can execute. Store the effective spec with the result so a later change to defaults cannot alter a replay.

Start with single-symbol consecutive hourly bars. Import CSV or Parquet via the existing data readers. Reject duplicate, missing or unordered bars and invalid OHLCV values before marking a version ready. Record parser/schema version, symbol, quote currency, coverage and row count. A replacement upload creates another version.

- **Single run:** execute one strategy against one immutable series.
- **Comparison:** run each selected strategy with its own account on the same bars. Preserve existing cash and buy-and-hold benchmark rules. Include DipDCA's unspent reserve and purchase wait in its result.
- **Window evaluation:** retain the CLI's fixed-parameter walk-forward semantics. Each test window starts with fresh capital and indicators. Record individual window boundaries and metrics. Any future fit-on-train optimizer needs a distinct mode and an explicit leakage test.

An order observes a close and can fill at the next open. Keep signal time, execution time, fill or skip reason, quantity, fee and resulting cash in the ledger. Define metric units in the output schema and serialize undefined ratios as null. HTML is a view over the stored results. The machine-readable result manifest references the spec, build, summary, checksums, ledger and equity series.

## Run lifecycle and HTTP surface

| Request | Result |
| --- | --- |
| POST /v1/datasets | Create a project-scoped upload and pending dataset version |
| POST /v1/datasets/{id}/complete | Queue validation. Expose ready or rejected with diagnostics |
| GET /v1/strategies | Return runnable strategies and parameter schemas for the selected engine build |
| POST /v1/runs | Validate a spec against a ready dataset and enqueue it. Return 202 and run ID |
| GET /v1/runs/{id} | Return state, progress, effective spec and summary or structured error |
| POST /v1/runs/{id}/cancel | Cancel a queued job or request cancellation of a running attempt |
| GET /v1/runs/{id}/artifacts | Return an authorized manifest and download links |

Bind idempotency keys to project and request hash. Repeating a submission returns the original run. Reusing a key with different input returns a conflict. Claim queued rows with a short transaction using SKIP LOCKED, an attempt token and an expiring lease. Execute outside the transaction. Heartbeat the lease and check cancellation at bounded intervals. Retry transient errors up to a configured attempt limit. Validation errors are terminal.

Upload artifacts under attempt-specific keys. In one transaction verify the active token and cancellation state, attach the manifest and mark the run succeeded. An expired worker cannot overwrite a newer result. Clean up unreferenced objects after a grace period. Apply project-level CPU concurrency and upload limits. Record queue delay, simulation duration and failure reason by engine build.

## Delivery and acceptance

1. **Extract the shared contract.** Route existing CLI commands through it. Compare the ledger, final equity and skipped orders against a pinned sample fixture for all five strategies. Cover next-open fills, UTC day boundaries and independent windows.
2. **Ship one end-to-end platform run.** Upload a dataset, submit a DCA or DipDCA job, poll the API and inspect the equity and ledger in the browser. Run the same spec locally and compare results. Verify project isolation, duplicate submissions, cancellation and recovery after worker loss.
3. **Expose comparisons and windowed runs.** Display individual accounts, benchmark policies and per-window metrics. Reproduce a saved run after changing catalog defaults. Add more workers only after measuring queue delay and memory per job.

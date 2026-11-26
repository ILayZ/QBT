import hashlib
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Annotated, Literal
from uuid import UUID, uuid4

from fastapi import FastAPI, Header, HTTPException, Query, Request, Response
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from psycopg.types.json import Jsonb
from pydantic import BaseModel, ConfigDict, Field
from starlette.concurrency import run_in_threadpool

from shared import ARTIFACTS, build_id, connect, initialize

MAX_BYTES = 8 * 1024 * 1024
LOCAL_RUN_SCOPE = "local"


class Options(BaseModel):
    model_config = ConfigDict(extra="forbid", allow_inf_nan=False)
    strategy: Literal["DCA", "DipDCA", "MomentumSMA", "VolAdjustedDCA", "RegimeSwitch", "all"] = "DCA"
    capital: float = Field(default=10000, gt=0)
    dca_interval: int = Field(default=24, gt=0, le=2**32 - 1)
    dca_amount: float = Field(default=100, gt=0)
    order_amount: float | None = Field(default=None, gt=0)
    daily_limit: float | None = Field(default=None, gt=0)
    min_notional: float | None = Field(default=None, gt=0)
    limit_includes_fees: bool = False
    dip_lookback: int = Field(default=168, ge=2)
    dip_percent: float = Field(default=5, gt=0, lt=100)
    fee: float = Field(default=0.001, ge=0, lt=1)
    slippage: float = Field(default=0.0005, ge=0, lt=1)
    risk_free: float = 0.03


class Submission(BaseModel):
    model_config = ConfigDict(extra="forbid")
    dataset: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
    options: Options = Field(default_factory=Options)


@asynccontextmanager
async def lifespan(app):
    initialize()
    app.state.build = build_id()
    yield


app = FastAPI(title="QBT local platform", lifespan=lifespan)


WEB = Path(__file__).resolve().parents[1] / "web"
app.mount("/ui", StaticFiles(directory=WEB), name="ui")


@app.get("/", include_in_schema=False)
def workspace():
    return FileResponse(WEB / "index.html")


@app.get("/v1/run-options")
def run_options():
    return Options.model_json_schema()


def store_dataset(content):
    dataset = hashlib.sha256(content).hexdigest()
    with connect() as db:
        db.execute("INSERT INTO qbt_datasets VALUES (%s, %s) ON CONFLICT DO NOTHING", (dataset, content))
    return {"id": dataset}


@app.post("/v1/datasets", status_code=201)
async def upload(request: Request):
    if request.headers.get("content-type", "").split(";")[0].strip() != "text/csv":
        raise HTTPException(415, "Expected text/csv")
    content = bytearray()
    async for chunk in request.stream():
        if len(content) + len(chunk) > MAX_BYTES:
            raise HTTPException(413, "CSV exceeds 8 MiB")
        content.extend(chunk)
    if not content:
        raise HTTPException(422, "Empty dataset")
    return await run_in_threadpool(store_dataset, bytes(content))


@app.post("/v1/runs", status_code=202)
def submit(submission: Submission, idempotency_key: Annotated[list[str] | None, Header()] = None):
    key = None
    if idempotency_key is not None:
        if len(idempotency_key) != 1:
            raise HTTPException(422, "Send exactly one Idempotency-Key header")
        key = idempotency_key[0]
        if not 1 <= len(key) <= 128 or any(not "!" <= char <= "~" or char == "," for char in key):
            raise HTTPException(422, "Idempotency-Key must contain 1–128 visible ASCII characters without commas")

    # Validated options include all defaults. JSONB equality ignores object key
    # order and equivalent numeric representations.
    options = Jsonb(submission.options.model_dump())
    scope = LOCAL_RUN_SCOPE if key is not None else None
    with connect() as db:
        row = db.execute("""
            INSERT INTO qbt_runs (id, dataset, options, build, idempotency_scope, idempotency_key)
            SELECT %s, id, %s, %s, %s, %s FROM qbt_datasets WHERE id = %s
            ON CONFLICT (idempotency_scope, idempotency_key) DO NOTHING
            RETURNING id, state
        """, (uuid4(), options, app.state.build, scope, key, submission.dataset)).fetchone()
        if row is None and key is not None:
            # At READ COMMITTED this statement sees the winner after the insert
            # waits for a simultaneous transaction to commit.
            row = db.execute("""
                SELECT id, state, dataset = %s AND options = %s AS matches
                FROM qbt_runs WHERE idempotency_scope = %s AND idempotency_key = %s
            """, (submission.dataset, options, scope, key)).fetchone()
            if row is not None and not row["matches"]:
                raise HTTPException(409, "Idempotency-Key already used for a different run request")
        if row is None:
            raise HTTPException(404, "Unknown dataset")
    return {"id": row["id"], "state": row["state"]}


@app.get("/v1/runs")
def list_runs(limit: Annotated[int, Query(ge=1, le=100)] = 20, before: UUID | None = None):
    with connect() as db:
        params = []
        where = ""
        if before is not None:
            anchor = db.execute("SELECT created_at FROM qbt_runs WHERE id = %s", (before,)).fetchone()
            if anchor is None:
                raise HTTPException(404, "Unknown history cursor")
            where = "WHERE (created_at, id) < (%s, %s)"
            params.extend((anchor["created_at"], before))
        rows = db.execute(f"""
            SELECT id, created_at, state, options->>'strategy' AS strategy,
                   options->'capital' AS capital, options->'daily_limit' AS daily_limit
            FROM qbt_runs {where}
            ORDER BY created_at DESC, id DESC LIMIT %s
        """, (*params, limit + 1)).fetchall()
    return {"runs": rows[:limit], "next": rows[limit - 1]["id"] if len(rows) > limit else None}


@app.get("/v1/runs/{run_id}")
def status(run_id: UUID):
    with connect() as db:
        row = db.execute("""
            SELECT id, dataset, options, build, state, error FROM qbt_runs WHERE id = %s
        """, (run_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "Unknown run")
        links = db.execute("SELECT name FROM qbt_artifacts WHERE run_id = %s ORDER BY name", (run_id,)).fetchall() if row["state"] == "succeeded" else []
    row["artifacts"] = {item["name"]: f"/v1/runs/{run_id}/artifacts/{item['name']}" for item in links}
    return row


@app.post("/v1/runs/{run_id}/cancel")
def cancel(run_id: UUID):
    with connect() as db:
        row = db.execute("SELECT state FROM qbt_runs WHERE id = %s FOR UPDATE", (run_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "Unknown run")
        if row["state"] in ("queued", "running"):
            db.execute("""
                UPDATE qbt_runs SET state = 'cancelled', attempt = NULL, expires_at = NULL
                WHERE id = %s
            """, (run_id,))
        elif row["state"] != "cancelled":
            raise HTTPException(409, "Run already finished")
    return {"id": run_id, "state": "cancelled"}


@app.get("/v1/runs/{run_id}/artifacts/{name}")
def download(run_id: UUID, name: str):
    if name not in ARTIFACTS:
        raise HTTPException(404, "Unknown artifact")
    with connect() as db:
        row = db.execute("SELECT content FROM qbt_artifacts WHERE run_id = %s AND name = %s", (run_id, name)).fetchone()
    if row is None:
        raise HTTPException(404, "Artifact unavailable")
    return Response(row["content"], media_type=ARTIFACTS[name], headers={"Content-Disposition": f'attachment; filename="{name}"'})

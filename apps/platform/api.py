import hashlib
from contextlib import asynccontextmanager
from typing import Annotated, Literal
from uuid import UUID, uuid4

from fastapi import FastAPI, HTTPException, Request, Response
from psycopg.types.json import Jsonb
from pydantic import BaseModel, ConfigDict, Field
from starlette.concurrency import run_in_threadpool

from shared import ARTIFACTS, build_id, connect, initialize

MAX_BYTES = 8 * 1024 * 1024


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
def submit(submission: Submission):
    run_id = uuid4()
    with connect() as db:
        row = db.execute("""
            INSERT INTO qbt_runs (id, dataset, options, build)
            SELECT %s, id, %s, %s FROM qbt_datasets WHERE id = %s
            RETURNING id, state
        """, (run_id, Jsonb(submission.options.model_dump()), app.state.build, submission.dataset)).fetchone()
        if row is None:
            raise HTTPException(404, "Unknown dataset")
    return row


@app.get("/v1/runs/{run_id}")
def status(run_id: UUID):
    with connect() as db:
        row = db.execute("""
            SELECT id, dataset, options, build, state, error FROM qbt_runs WHERE id = %s
        """, (run_id,)).fetchone()
    if row is None:
        raise HTTPException(404, "Unknown run")
    row["artifacts"] = {name: f"/v1/runs/{run_id}/artifacts/{name}" for name in ARTIFACTS} if row["state"] == "succeeded" else {}
    return row


@app.get("/v1/runs/{run_id}/artifacts/{name}")
def download(run_id: UUID, name: str):
    if name not in ARTIFACTS:
        raise HTTPException(404, "Unknown artifact")
    with connect() as db:
        row = db.execute("SELECT content FROM qbt_artifacts WHERE run_id = %s AND name = %s", (run_id, name)).fetchone()
    if row is None:
        raise HTTPException(404, "Artifact unavailable")
    return Response(row["content"], media_type=ARTIFACTS[name], headers={"Content-Disposition": f'attachment; filename="{name}"'})

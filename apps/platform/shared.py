import hashlib
import os
from pathlib import Path

import psycopg
from psycopg.rows import dict_row

BINARY = Path(os.environ.get("QBT_BINARY", "target/release/qbt")).resolve()
ARTIFACTS = {
    "report.html": "text/html",
    "orders.csv": "text/csv",
    "summary.json": "application/json",
    "equity.json": "application/json",
    "manifest.json": "application/json",
}


def build_id():
    with BINARY.open("rb") as binary:
        return hashlib.file_digest(binary, "sha256").hexdigest()


def connect(**kwargs):
    return psycopg.connect(os.environ["DATABASE_URL"], row_factory=dict_row, **kwargs)


def initialize():
    with connect() as db:
        # API and worker may start together on a new database.
        db.execute("SELECT pg_advisory_xact_lock(5172601)")
        db.execute("""
            CREATE TABLE IF NOT EXISTS qbt_datasets (
                id text PRIMARY KEY,
                csv bytea NOT NULL
            );
            ALTER TABLE qbt_datasets
                ADD COLUMN IF NOT EXISTS format text NOT NULL DEFAULT 'csv'
                    CHECK (format IN ('csv', 'parquet')),
                ADD COLUMN IF NOT EXISTS symbol text,
                ADD COLUMN IF NOT EXISTS quote_currency text,
                ADD COLUMN IF NOT EXISTS validation jsonb;
            CREATE TABLE IF NOT EXISTS qbt_runs (
                id uuid PRIMARY KEY,
                dataset text NOT NULL REFERENCES qbt_datasets(id),
                options jsonb NOT NULL,
                build text NOT NULL,
                created_at timestamptz NOT NULL DEFAULT now(),
                state text NOT NULL DEFAULT 'queued',
                attempt uuid,
                attempts integer NOT NULL DEFAULT 0,
                expires_at timestamptz,
                error text
            );
            ALTER TABLE qbt_runs
                ADD COLUMN IF NOT EXISTS idempotency_scope text,
                ADD COLUMN IF NOT EXISTS idempotency_key text;
            CREATE UNIQUE INDEX IF NOT EXISTS qbt_runs_idempotency
                ON qbt_runs (idempotency_scope, idempotency_key);
            CREATE INDEX IF NOT EXISTS qbt_runs_history ON qbt_runs (created_at DESC, id DESC);
            CREATE TABLE IF NOT EXISTS qbt_artifacts (
                run_id uuid NOT NULL REFERENCES qbt_runs(id),
                name text NOT NULL,
                content bytea NOT NULL,
                PRIMARY KEY (run_id, name)
            )
        """)

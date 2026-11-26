# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends libfontconfig1-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY apps/cli/ apps/cli/
COPY crates/qbt-engine/ crates/qbt-engine/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --package qbt-cli --jobs 2 \
    && install -Dm0555 target/release/qbt /out/qbt

FROM python:3.12-slim-bookworm
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 QBT_BINARY=/usr/local/bin/qbt
WORKDIR /opt/qbt
RUN apt-get update && apt-get install -y --no-install-recommends libfontconfig1 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
COPY apps/platform/requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY --from=build /out/qbt /usr/local/bin/qbt
COPY apps/platform/ apps/platform/
COPY apps/web/ apps/web/
USER 10001:10001

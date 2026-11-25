# qbacktest

**A fast, scriptable, reproducible backtesting engine for retail traders and quant hobbyists.**

Run a strategy over your own OHLCV data in seconds, get risk-adjusted metrics and a
self-contained HTML report — no cloud subscription, no lock-in, no waiting.

## Why

Most retail traders who want to backtest a strategy are forced into either:

- Heavy cloud platforms (paid, slow, closed source), or
- Complex Python notebooks (dependency hell, slow on large data).

`qbacktest` is a single, self-contained CLI binary. It reads CSV or Parquet, runs a
strategy, computes standard performance and risk metrics, and writes an HTML report.
Fast, reproducible, and yours.

## Features

- **Single static binary** — no runtime, no package manager, no virtualenv.
- **Real backtesting** — configurable initial capital, trading fee, slippage, risk-free rate.
- **Built-in strategies:** DCA (dollar-cost averaging), MomentumSMA, VolAdjustedDCA.
- **Windowed evaluation** — each test window starts with fresh capital and indicators. The CLI uses fixed parameters. It reports mean window return and the fraction of profitable windows.
- **Metrics:** total/annualized return, Sharpe, Sortino, max drawdown, win rate, profit factor, volatility.
- **HTML report** — self-contained, shareable, with charts.
- **Fast** — Rust + Polars; handles large datasets without notebooks.

## Install

Build from source (requires Rust stable):

```bash
git clone <repo-url> qbacktest
cd qbacktest
cargo build --release
# binary at: target/release/qbacktest
```

## Quick start

```bash
# List available strategies
qbacktest strategies --verbose

# Run a DCA backtest on hourly OHLCV data
qbacktest run \
  --input data/sample_btc_1h.csv \
  --strategy DCA \
  --dca-interval 24 \
  --dca-amount 100 \
  --capital 10000 \
  --fee 0.001 \
  --output report.html

# Walk-forward validation of a momentum strategy
qbacktest walk-forward \
  --input data/sample_btc_1h.csv \
  --strategy MomentumSMA \
  --train 1 --test 1 --step 1 \
  --output wf_report.html
```

## Input data format

CSV with header row and these columns:

```csv
timestamp,open,high,low,close,volume
2024-01-01T00:00:00Z,42000.00,42014.52,41968.55,41977.89,1280.79
```

- `timestamp`: ISO 8601 UTC datetime. Backtests require consecutive hourly bars in chronological order.
- A signal uses the current close and fills at the following open when funds and position allow. Equity is valued at each close. The final close signal expires.
- `open`, `high`, `low`, `close`, `volume`: floating point values.
- Parquet is also supported (same column names; `timestamp` as datetime).

A ready-to-use 180-day sample dataset is included at `data/sample_btc_1h.csv`.

## Commands

### `run`

| Flag | Default | Description |
|------|---------|-------------|
| `--input` | *(required)* | Input CSV/Parquet (OHLCV) |
| `--output` | `report.html` | Output HTML report path |
| `--strategy` | `DCA` | `DCA`, `MomentumSMA`, or `VolAdjustedDCA` |
| `--dca-interval` | `24` | DCA interval in hours |
| `--dca-amount` | `100` | DCA amount per interval |
| `--capital` | `10000` | Initial capital |
| `--fee` | `0.001` | Trading fee (fraction) |
| `--slippage` | `0.0005` | Slippage (fraction) |
| `--risk-free` | `0.03` | Annual risk-free rate |

### `walk-forward`

| Flag | Default | Description |
|------|---------|-------------|
| `--input` | *(required)* | Input CSV/Parquet |
| `--strategy` | `MomentumSMA` | Fixed strategy to evaluate |
| `--train` | `12` | History offset (30-day months) |
| `--test` | `6` | Test window (months) |
| `--step` | `6` | Step (months) |
| `--output` | `walk_forward_report.html` | Output HTML |

### `report`

Generate an HTML report from a JSON file of results.

### `strategies`

List available strategies. Use `--verbose` for parameter details.

## Monetization (planned)

- **Free:** open-source core engine, basic strategies, HTML reports.
- **Pro (planned):** additional strategy library, scheduled/automated report generation,
  multi-symbol portfolio backtests, priority support.

## License

Open-source core. See `LICENSE` (TBD — MIT planned).

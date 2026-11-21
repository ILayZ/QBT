# qbacktest — Landing Page Draft

> **STATUS: DRAFT. NOT PUBLISHED.**
> Publishing, account creation, and outreach require explicit human authorization (blocker B2).
> This is local preparation only so launch can happen immediately after approval.

---

# Backtest any trading strategy in seconds. No cloud. No lock-in.

**qbacktest** is a single, self-contained command-line engine that runs your strategy over
your own OHLCV data, computes real risk-adjusted metrics, and writes a shareable HTML report —
straight from your terminal.

No subscriptions. No account. No uploading your data to someone else's server.

## Why traders switch to qbacktest

| Cloud platforms | Python notebooks | **qbacktest** |
|---|---|---|
| Paid, slow, closed source | Dependency hell, slow on big data | **Single static binary, zero runtime** |
| Your data leaves your machine | Your data stays local but fragile | **Your data never leaves your machine** |
| Rigid, preset strategies | You write everything yourself | **Real engine + built-in strategies + walk-forward** |
| Opaque "AI" metrics | DIY, easy to get wrong | **Standard metrics: Sharpe, Sortino, Max DD, Profit Factor** |

## What you get

- **One static binary.** `qbacktest run --input data.csv` — that's it.
- **Real backtesting.** Configurable capital, trading fee, slippage, risk-free rate.
- **Built-in strategies.** DCA, MomentumSMA, VolAdjustedDCA — plus a clear extension point.
- **Walk-forward validation.** Train/test splits with out-of-sample consistency scoring, so you
  aren't fooling yourself with in-sample curve-fit.
- **Standard risk metrics.** Total & annualized return, Sharpe, Sortino, max drawdown, win rate,
  profit factor, volatility.
- **Self-contained HTML report.** Charts and metrics in one file you can send to anyone.
- **CSV or Parquet in.** Works with data you already have.

## Try it

```bash
# List strategies
qbacktest strategies --verbose

# Run a DCA backtest on hourly BTC data
qbacktest run --input data/sample_btc_1h.csv --strategy DCA \
  --dca-interval 24 --dca-amount 100 --capital 10000 --output report.html

# Walk-forward validation of a momentum strategy
qbacktest walk-forward --input data/sample_btc_1h.csv --strategy MomentumSMA \
  --train 12 --test 6 --step 6 --output wf_report.html
```

## Pricing (planned)

- **Free:** open-source core engine, built-in strategies, HTML reports.
- **Pro (planned):** extended strategy library, automated/scheduled reports, multi-symbol
  portfolio backtests, priority support.

---

## Product description (short, for marketplaces/directories)

**qbacktest** is a fast, scriptable, reproducible backtesting engine for retail traders and
quant hobbyists. A single Rust CLI binary reads CSV or Parquet, runs a strategy (DCA, MomentumSMA,
VolAdjustedDCA, or your own), computes standard risk-adjusted metrics (Sharpe, Sortino, max
drawdown, profit factor), and outputs a self-contained HTML report — no cloud, no subscriptions,
no Python environment. Data stays on your machine.

## Tagline options

- "Backtest in seconds. Own your data."
- "The backtesting engine that respects your data."
- "Reproducible backtests, one binary, zero lock-in."

# QBT — Marketplace / Directory Listing Pack

> **STATUS: READY. NOT PUBLISHED.**
> Publishing, account creation, and outreach require explicit human authorization (blocker B2).
> Local preparation only so launch can happen immediately after approval.

---

## Short product description (for marketplaces / directories)

**QBT** is a fast, scriptable, reproducible backtesting engine for retail traders and
quant hobbyists. A single Rust CLI binary reads CSV or Parquet, runs a strategy (DCA, MomentumSMA,
VolAdjustedDCA, or your own), computes standard risk-adjusted metrics (Sharpe, Sortino, max
drawdown, profit factor), and outputs a self-contained HTML report — no cloud, no subscriptions,
no Python environment. Data stays on your machine.

## Tagline options

- "Backtest in seconds. Own your data."
- "The backtesting engine that respects your data."
- "Reproducible backtests, one binary, zero lock-in."

## Categories

- Developer Tools / Finance / Quantitative Analysis / Trading

## Key selling points (bullets)

- Single static binary — no runtime, no Python env, no dependency hell
- Standard risk metrics: Sharpe, Sortino, max drawdown, win rate, profit factor
- Built-in strategies (DCA, MomentumSMA, VolAdjustedDCA) plus a clear extension point
- Walk-forward validation with train/test splits and out-of-sample consistency scoring
- Self-contained HTML report — shareable, no notebook needed
- CSV or Parquet input; your data never leaves your machine
- Open-source core (MIT) with a paid Pro tier planned

## Canonical landing page

The single canonical landing page is `docs/landing-page.html` (HTML, deployable).
This file holds only marketplace-ready text. Do not maintain a second landing-page draft.

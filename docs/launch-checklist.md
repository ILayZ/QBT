# QBT — Launch Checklist

Purpose: take QBT from "working binary" to "live and acquiring customers" with
the least friction once the release build and E2E pass. Items are grouped by whether
they are autonomous (no approval) or require human authorization (B2).

## A. Pre-launch verification (AUTONOMOUS — runs after build completes)

- [ ] 1. `cargo build --release` completes; `target/release/qbt` exists.
- [ ] 2. `scripts/e2e_verify.sh` passes (binary, strategies --verbose, DCA run,
        walk-forward, HTML report present + non-trivial, metrics sane).
- [ ] 3. Fix B7/P1 — walk_forward must slice the OOS window (backtest/mod.rs:
        `let oos_series = &series;` currently uses the FULL series). Core logic fix;
        do NOT edit source mid-build.
- [ ] 4. Fix B7/P2 — BacktestResult.strategy_name set to String::new(); HTML report
        shows blank "Strategy" row. Cosmetic; same constraint.
- [ ] 5. Re-run E2E after fixes 3-4; all checks green.
- [ ] 6. README final pass — verify usage examples match actual CLI flags
        (run, strategies, walk-forward subcommands; --input/--strategy/--output).
- [ ] 7. `git tag v0.1.0` (after E2E green + fixes).

## B. Launch assets (DONE — prepared 2026-11-21)

- [x] docs/landing-page.html — canonical landing page (commit 8b509e7)
- [x] marketing/product-description.md — marketplace description pack (commit 8b509e7)
- [x] LICENSE — MIT (commit 758732a)
- [x] README.md — feature list + usage (needs final flag pass in A6)
- [x] Free/Pro pricing table — Free: local CLI / Pro: report + strategy packs
        (commit 84bb7f0, pending demand validation)

## C. Distribution (REQUIRES HUMAN APPROVAL — B2)

- [ ] 8. Publish binary to GitHub Releases (needs remote repo + token).
- [ ] 9. Host landing page (GitHub Pages, free) — publish action.
- [ ] 10. List in product directories / developer communities (accounts required).
- [ ] 11. Set up payment processing (Stripe / Gumroad / etc.) — account + approval.

## D. Customer acquisition (REQUIRES HUMAN APPROVAL — B2)

- [ ] 12. Prospect-level validation — contact traders/forums/subreddits that already
        discuss backtesting pain (TradingView €12.95–59.95/mo, VectorBT free-core
        evidence from 2026-11-21).
- [ ] 13. Authorized outreach: product demos, trial activations, first paying customer.

## E. Monetization model (REFERENCE)

- Free tier: local CLI backtesting, no cloud, MIT-licensed core.
- Pro tier (paid): HTML report generation, premium strategy packs, hosted
  walk-forward, priority support. Price point to validate in prospect outreach.

## Status

- Blocked on: release build (in progress, healthy), then E2E, then fixes B7/P1-P2.
- Nothing in section C/D may be executed until B2 authorization is granted.

#!/usr/bin/env bash
# qbacktest end-to-end verification harness
# Runs the release binary against sample data and asserts tangible outputs.
# Usage: ./scripts/e2e_verify.sh [path-to-binary]
set -uo pipefail

BIN="${1:-target/release/qbacktest}"
DATA="data/sample_btc_1h.csv"
OUT_DIR=".e2e"
mkdir -p "$OUT_DIR"

pass=0; fail=0
ok()   { echo "  [PASS] $1"; pass=$((pass+1)); }
bad()  { echo "  [FAIL] $1"; fail=$((fail+1)); }

echo "== qbacktest E2E =="

# 0. Binary exists and is executable
if [[ -x "$BIN" ]]; then ok "binary present: $BIN"; else bad "binary missing: $BIN (build not finished?)"; fi
[[ -f "$DATA" ]] && ok "sample data present ($(wc -l < "$DATA") lines)" || bad "sample data missing"

# 1. strategies
if [[ -x "$BIN" ]]; then
  if "$BIN" strategies --verbose > "$OUT_DIR/strategies.txt" 2>"$OUT_DIR/strategies.err"; then
    ok "strategies --verbose exit 0"
    grep -qiE "DCA|Momentum|Vol" "$OUT_DIR/strategies.txt" && ok "strategies listed" || bad "no known strategies in output"
  else
    bad "strategies --verbose failed (see .e2e/strategies.err)"
  fi
fi

# 2. run (DCA) -> HTML report
if [[ -x "$BIN" ]]; then
  if "$BIN" run --input "$DATA" --strategy DCA --dca-interval 24 --dca-amount 100 \
      --capital 10000 --fee 0.001 --output "$OUT_DIR/report.html" > "$OUT_DIR/run.txt" 2>"$OUT_DIR/run.err"; then
    ok "run DCA exit 0"
    if [[ -s "$OUT_DIR/report.html" ]] && grep -qiE "<html|<!DOCTYPE|svg" "$OUT_DIR/report.html"; then
      ok "HTML report produced (bytes: $(wc -c < "$OUT_DIR/report.html"))"
    else
      bad "report.html missing/empty/non-HTML"
    fi
    grep -qiE "return|sharpe|drawdown|win" "$OUT_DIR/run.txt" && ok "metrics present in stdout" || echo "  [note] no metric keywords in stdout"
  else
    bad "run DCA failed (see .e2e/run.err)"
  fi
fi

# 3. walk-forward (MomentumSMA)
# NOTE: walk_forward requires train_bars+test_bars <= total_bars, where each "month"
#   = 30 days * 24 hourly bars = 720 bars. Sample data = 4320 bars (~180 days).
#   train=1/test=1/step=1 month => 720/720/720 bars => loop yields 5 OOS periods.
#   Do NOT use train=12/test=6 (8640+4320=12960 bars) here — it exceeds 4320 and
#   walk_forward bails with "Insufficient data for walk-forward with current window sizes".
if [[ -x "$BIN" ]]; then
  if "$BIN" walk-forward --input "$DATA" --strategy MomentumSMA --train 1 --test 1 --step 1 \
      --output "$OUT_DIR/walk.html" > "$OUT_DIR/walk.txt" 2>"$OUT_DIR/walk.err"; then
    ok "walk-forward exit 0"
    [[ -s "$OUT_DIR/walk.html" ]] && ok "walk-forward HTML produced" || bad "walk.html missing/empty"
  else
    bad "walk-forward failed (see .e2e/walk.err)"
  fi
fi

# One $100 buy at the second open ($50), $1 fee, final close $55:
# $899 in cash + 2 units * $55 = $1009.
if [[ -x "$BIN" ]]; then
  cat > "$OUT_DIR/reference.csv" <<'CSV'
timestamp,open,high,low,close,volume
2024-01-01T00:00:00Z,40.0,40.0,40.0,40.0,1.0
2024-01-01T01:00:00Z,50.0,55.0,50.0,55.0,1.0
CSV
  if "$BIN" run --input "$OUT_DIR/reference.csv" --strategy DCA --dca-interval 1 \
      --capital 1000 --dca-amount 100 --fee 0.01 --slippage 0 --output "$OUT_DIR/reference.html" &&
      grep -Fq 'Final Value</b></td><td>$1009.00</td>' "$OUT_DIR/reference.html"; then
    ok "reference DCA final value"
  else
    bad "reference DCA final value"
  fi
fi

echo "== RESULT: $pass passed, $fail failed =="
[[ "$fail" -eq 0 ]] && exit 0 || exit 1

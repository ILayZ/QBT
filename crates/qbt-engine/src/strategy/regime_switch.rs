use super::catalog::DEFAULT_ENTRY_AMOUNT;
use super::{Regime, SELL_ALL, Signal, detect_regime, sma};

pub(super) fn signals(
    closes: &[f64],
    fast_sma: u32,
    slow_sma: u32,
    buy_regime: Regime,
    sell_regime: Regime,
) -> Vec<Signal> {
    let fast_vals = sma(closes, fast_sma as usize);
    let slow_vals = sma(closes, slow_sma as usize);
    let mut signals = Vec::with_capacity(closes.len());
    for i in 0..closes.len() {
        let regime = detect_regime(&fast_vals, &slow_vals, i);
        if regime == buy_regime {
            signals.push(Signal::Buy {
                amount: DEFAULT_ENTRY_AMOUNT,
            });
        } else if regime == sell_regime {
            signals.push(Signal::Sell { amount: SELL_ALL });
        } else {
            signals.push(Signal::Hold);
        }
    }
    signals
}

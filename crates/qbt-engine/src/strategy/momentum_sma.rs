use super::catalog::DEFAULT_ENTRY_AMOUNT;
use super::{SELL_ALL, Signal, sma};

pub(super) fn signals(
    closes: &[f64],
    lookback: u32,
    buy_threshold: f64,
    sell_threshold: f64,
) -> Vec<Signal> {
    let sma_vals = sma(closes, lookback as usize);
    let mut signals = Vec::with_capacity(closes.len());
    for i in 0..closes.len() {
        let price = closes[i];
        let average = sma_vals[i];
        if price > average * (1.0 + buy_threshold) {
            signals.push(Signal::Buy {
                amount: DEFAULT_ENTRY_AMOUNT,
            });
        } else if price < average * (1.0 - sell_threshold) {
            signals.push(Signal::Sell { amount: SELL_ALL });
        } else {
            signals.push(Signal::Hold);
        }
    }
    signals
}

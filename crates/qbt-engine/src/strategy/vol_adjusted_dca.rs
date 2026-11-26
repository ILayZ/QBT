use super::{Signal, rolling_vol};

pub(super) fn signals(
    closes: &[f64],
    base_amount: f64,
    vol_lookback: u32,
    vol_multiplier_max: f64,
) -> Vec<Signal> {
    let vols = rolling_vol(closes, vol_lookback as usize);
    let mut signals = Vec::with_capacity(closes.len());
    for vol in vols {
        if vol.is_nan() {
            signals.push(Signal::Hold);
        } else {
            let mult = (vol * 10.0).min(vol_multiplier_max);
            let amount = base_amount * (1.0 + mult);
            signals.push(Signal::Buy { amount });
        }
    }
    signals
}

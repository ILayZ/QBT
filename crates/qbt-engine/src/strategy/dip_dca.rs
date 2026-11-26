use super::Signal;

pub(super) fn signals(closes: &[f64], lookback_hours: usize, drop_fraction: f64) -> Vec<Signal> {
    let mut signals = vec![Signal::Hold; closes.len()];
    // Keep candidate peak indices in descending price order.
    let mut peaks = std::collections::VecDeque::<usize>::new();
    for (i, &close) in closes.iter().enumerate() {
        let start = (i + 1).saturating_sub(lookback_hours);
        while peaks.front().is_some_and(|&j| j < start) {
            peaks.pop_front();
        }
        while peaks.back().is_some_and(|&j| closes[j] <= close) {
            peaks.pop_back();
        }
        peaks.push_back(i);
        if i + 1 >= lookback_hours && close <= closes[peaks[0]] * (1.0 - drop_fraction) {
            signals[i] = Signal::BuyReserve;
        }
    }
    signals
}

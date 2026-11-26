use super::Signal;

pub(super) fn signals(bars: usize, interval_hours: u32, amount: f64) -> Vec<Signal> {
    // Emit a signal for EVERY bar (Buy on interval boundaries, Hold otherwise)
    // so the signal vector is the same length as the price series.
    let interval = interval_hours as usize;
    if interval == 0 {
        // Return all Hold signals rather than panic on division by zero
        return vec![Signal::Hold; bars];
    }
    let mut signals = Vec::with_capacity(bars);
    for i in 0..bars {
        if i % interval == 0 {
            signals.push(Signal::Buy { amount });
        } else {
            signals.push(Signal::Hold);
        }
    }
    signals
}

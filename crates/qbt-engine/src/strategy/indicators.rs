use super::Regime;

/// Simple SMA calculator
pub fn sma(series: &[f64], window: usize) -> Vec<f64> {
    if window == 0 || series.len() < window {
        return vec![f64::NAN; series.len()];
    }
    let mut result = vec![f64::NAN; series.len()];
    let mut sum: f64 = 0.0;
    for i in 0..series.len() {
        sum += series[i];
        if i >= window {
            sum -= series[i - window];
        }
        if i >= window - 1 {
            result[i] = sum / window as f64;
        }
    }
    result
}

/// Volatility (rolling standard deviation of returns)
pub fn rolling_vol(series: &[f64], window: usize) -> Vec<f64> {
    if window == 0 || series.len() < 2 {
        return vec![f64::NAN; series.len()];
    }
    let rets: Vec<f64> = series.windows(2).map(|w| (w[1] - w[0]) / w[0]).collect();
    let mut result = vec![f64::NAN; series.len()];
    for i in window..series.len() {
        let chunk = &rets[i - window..i];
        let mean = chunk.iter().sum::<f64>() / window as f64;
        let variance = chunk.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / window as f64;
        result[i] = variance.sqrt();
    }
    result
}

/// Detect market regime based on SMA crossovers
pub fn detect_regime(fast_sma: &[f64], slow_sma: &[f64], idx: usize) -> Regime {
    if idx >= fast_sma.len() || fast_sma[idx].is_nan() || slow_sma[idx].is_nan() {
        return Regime::Sideways;
    }
    let fast = fast_sma[idx];
    let slow = slow_sma[idx];
    let diff = (fast - slow) / slow;
    if diff > 0.02 {
        Regime::Bull
    } else if diff < -0.02 {
        Regime::Bear
    } else {
        Regime::Sideways
    }
}

use crate::data::PriceSeries;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Strategy definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Strategy {
    /// Dollar-cost averaging at fixed intervals
    DCA { interval_hours: u32, amount: f64 },
    /// Momentum-based (buy when price > SMA, sell when price < SMA)
    MomentumSMA {
        lookback: u32,
        buy_threshold: f64,
        sell_threshold: f64,
    },
    /// Volatility-adjusted DCA (increase buys when volatility high)
    VolAdjustedDCA {
        base_amount: f64,
        vol_lookback: u32,
        vol_multiplier_max: f64,
    },
    /// Regime switching (switch between buy/sell based on market regime)
    RegimeSwitch {
        fast_sma: u32,
        slow_sma: u32,
        buy_regime: Regime,
        sell_regime: Regime,
    },
    /// Custom strategy (user-provided code)
    Custom {
        name: String,
        config: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Regime {
    Bull,
    Bear,
    Sideways,
}

impl fmt::Display for Regime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Regime::Bull => write!(f, "Bull"),
            Regime::Bear => write!(f, "Bear"),
            Regime::Sideways => write!(f, "Sideways"),
        }
    }
}

/// Strategy signal (buy/sell/hold)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Signal {
    Buy { amount: f64 },
    Sell { amount: f64 },
    Hold,
}

impl Signal {
    pub fn is_action(&self) -> bool {
        !matches!(self, Signal::Hold)
    }
}

/// Sentinel amount meaning "sell the entire position" (full exit).
/// The backtester treats any Sell amount >= f64::MAX / 2 as a full-position exit.
pub const SELL_ALL: f64 = f64::MAX / 4.0;

/// Strategy trait (for future extensibility)
pub trait StrategyEngine: Send + Sync {
    fn name(&self) -> &str;
    fn generate_signals(&self, series: &PriceSeries, params: &HashMap<String, f64>) -> Vec<Signal>;
}

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

impl Strategy {
    pub fn name(&self) -> &str {
        match self {
            Strategy::DCA { .. } => "DCA",
            Strategy::MomentumSMA { .. } => "MomentumSMA",
            Strategy::VolAdjustedDCA { .. } => "VolAdjustedDCA",
            Strategy::RegimeSwitch { .. } => "RegimeSwitch",
            Strategy::Custom { name, .. } => name.as_str(),
        }
    }

    pub fn generate_signals(&self, series: &PriceSeries) -> Vec<Signal> {
        let closes = series.closes();
        match self {
            Strategy::DCA {
                interval_hours,
                amount,
            } => {
                // Emit a signal for EVERY bar (Buy on interval boundaries, Hold otherwise)
                // so the signal vector is the same length as the price series.
                let interval = *interval_hours as usize;
                if interval == 0 {
                    // Return all Hold signals rather than panic on division by zero
                    return vec![Signal::Hold; closes.len()];
                }
                let mut signals = Vec::with_capacity(closes.len());
                for i in 0..closes.len() {
                    if i % interval == 0 {
                        signals.push(Signal::Buy { amount: *amount });
                    } else {
                        signals.push(Signal::Hold);
                    }
                }
                signals
            }
            Strategy::MomentumSMA {
                lookback,
                buy_threshold,
                sell_threshold,
            } => {
                let sma_vals = sma(&closes, *lookback as usize);
                let mut signals = Vec::with_capacity(closes.len());
                let mut in_position = false;
                for i in 0..closes.len() {
                    if i < *lookback as usize {
                        signals.push(Signal::Hold);
                        continue;
                    }
                    // CF-019: Use prior-bar close for signal generation (causal)
                    // Signal at bar i uses close[i-1] and SMA[i-1] (known at bar i's open)
                    let price = closes[i - 1];
                    let s = sma_vals[i - 1];
                    if !in_position && price > s * (1.0 + buy_threshold) {
                        signals.push(Signal::Buy { amount: 100.0 }); // placeholder
                        in_position = true;
                    } else if in_position && price < s * (1.0 - sell_threshold) {
                        signals.push(Signal::Sell { amount: SELL_ALL });
                        in_position = false;
                    } else {
                        signals.push(Signal::Hold);
                    }
                }
                signals
            }
            Strategy::VolAdjustedDCA {
                base_amount,
                vol_lookback,
                vol_multiplier_max,
            } => {
                let vols = rolling_vol(&closes, *vol_lookback as usize);
                let mut signals = Vec::with_capacity(closes.len());
                for vol in vols {
                    if vol.is_nan() {
                        signals.push(Signal::Hold);
                    } else {
                        let mult = (vol * 10.0).min(*vol_multiplier_max);
                        let amount = base_amount * (1.0 + mult);
                        signals.push(Signal::Buy { amount });
                    }
                }
                signals
            }
            Strategy::RegimeSwitch {
                fast_sma,
                slow_sma,
                buy_regime,
                sell_regime,
            } => {
                let fast_vals = sma(&closes, *fast_sma as usize);
                let slow_vals = sma(&closes, *slow_sma as usize);
                let mut signals = Vec::with_capacity(closes.len());
                let mut position = 0.0; // 0 = no position, 1 = full
                for i in 0..closes.len() {
                    // CF-019: Use prior-bar regime for signal generation (causal)
                    // Regime at bar i uses SMA values from bar i-1
                    let regime = if i > 0 {
                        detect_regime(&fast_vals, &slow_vals, i - 1)
                    } else {
                        Regime::Sideways
                    };
                    if regime == *buy_regime && position == 0.0 {
                        signals.push(Signal::Buy { amount: 100.0 });
                        position = 1.0;
                    } else if regime == *sell_regime && position == 1.0 {
                        signals.push(Signal::Sell { amount: SELL_ALL });
                        position = 0.0;
                    } else {
                        signals.push(Signal::Hold);
                    }
                }
                signals
            }
            Strategy::Custom { .. } => {
                // Placeholder for custom strategies
                vec![Signal::Hold; closes.len()]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Bar;
    use chrono::{TimeZone, Utc};

    fn make_series(prices: Vec<f64>) -> PriceSeries {
        let bars: Vec<Bar> = prices
            .iter()
            .enumerate()
            .map(|(i, p)| Bar {
                timestamp: Utc.timestamp_opt(i as i64, 0).unwrap(),
                open: *p,
                high: *p,
                low: *p,
                close: *p,
                volume: 1000.0,
            })
            .collect();
        PriceSeries {
            symbol: "TEST".into(),
            bars,
        }
    }

    #[test]
    fn test_sma() {
        let prices = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&prices, 3);
        assert!(result[0].is_nan());
        assert!(result[1].is_nan());
        assert_eq!(result[2], 2.0);
        assert_eq!(result[3], 3.0);
        assert_eq!(result[4], 4.0);
    }

    #[test]
    fn test_rolling_vol() {
        let prices = vec![100.0, 110.0, 105.0, 115.0, 110.0];
        let vol = rolling_vol(&prices, 2);
        assert!(vol[0].is_nan());
        assert!(vol[1].is_nan());
        assert!(!vol[2].is_nan());
    }

    #[test]
    fn test_dca_signals() {
        let prices: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
        let series = make_series(prices);
        let strat = Strategy::DCA {
            interval_hours: 3,
            amount: 50.0,
        };
        let signals = strat.generate_signals(&series);
        let buys = signals
            .iter()
            .filter(|s| matches!(s, Signal::Buy { .. }))
            .count();
        assert_eq!(buys, 4); // 0, 3, 6, 9
    }

    #[test]
    fn test_momentum_signals() {
        let prices = vec![100.0, 101.0, 102.0, 110.0, 105.0, 100.0];
        let series = make_series(prices);
        let strat = Strategy::MomentumSMA {
            lookback: 3,
            buy_threshold: 0.01,
            sell_threshold: 0.01,
        };
        let signals = strat.generate_signals(&series);
        assert_eq!(signals.len(), 6);
        // Should have at least one buy and one sell
        let buys = signals
            .iter()
            .filter(|s| matches!(s, Signal::Buy { .. }))
            .count();
        assert!(buys > 0);
    }
}

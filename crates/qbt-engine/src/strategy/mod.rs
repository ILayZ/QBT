mod dca;
mod dip_dca;
mod indicators;
mod momentum_sma;
mod regime_switch;
mod vol_adjusted_dca;

pub use indicators::{detect_regime, rolling_vol, sma};

use crate::data::PriceSeries;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Strategy definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Strategy {
    /// Dollar-cost averaging at fixed intervals
    DCA { interval_hours: u32, amount: f64 },
    /// Buy from a cash reserve on a drawdown from the trailing closing high.
    DipDCA {
        daily_allocation: f64,
        lookback_hours: usize,
        drop_fraction: f64,
    },
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
    BuyReserve,
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
pub const SELL_ALL: f64 = f64::MAX;

/// Strategy trait (for future extensibility)
pub trait StrategyEngine: Send + Sync {
    fn name(&self) -> &str;
    fn generate_signals(&self, series: &PriceSeries, params: &HashMap<String, f64>) -> Vec<Signal>;
}

impl Strategy {
    pub fn name(&self) -> &str {
        match self {
            Strategy::DCA { .. } => "DCA",
            Strategy::DipDCA { .. } => "DipDCA",
            Strategy::MomentumSMA { .. } => "MomentumSMA",
            Strategy::VolAdjustedDCA { .. } => "VolAdjustedDCA",
            Strategy::RegimeSwitch { .. } => "RegimeSwitch",
            Strategy::Custom { name, .. } => name.as_str(),
        }
    }

    pub fn generate_signals(&self, series: &PriceSeries) -> Vec<Signal> {
        match self {
            Strategy::DipDCA {
                lookback_hours,
                drop_fraction,
                ..
            } => dip_dca::signals(&series.closes(), *lookback_hours, *drop_fraction),
            Strategy::DCA {
                interval_hours,
                amount,
            } => dca::signals(series.len(), *interval_hours, *amount),
            Strategy::MomentumSMA {
                lookback,
                buy_threshold,
                sell_threshold,
            } => {
                momentum_sma::signals(&series.closes(), *lookback, *buy_threshold, *sell_threshold)
            }
            Strategy::VolAdjustedDCA {
                base_amount,
                vol_lookback,
                vol_multiplier_max,
            } => vol_adjusted_dca::signals(
                &series.closes(),
                *base_amount,
                *vol_lookback,
                *vol_multiplier_max,
            ),
            Strategy::RegimeSwitch {
                fast_sma,
                slow_sma,
                buy_regime,
                sell_regime,
            } => regime_switch::signals(
                &series.closes(),
                *fast_sma,
                *slow_sma,
                *buy_regime,
                *sell_regime,
            ),
            Strategy::Custom { .. } => {
                // Placeholder for custom strategies
                vec![Signal::Hold; series.len()]
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

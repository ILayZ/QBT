use crate::backtest::{BacktestConfig, BacktestResult, Backtester, BuyBudget, OrderEvent};
use crate::data::PriceSeries;
use crate::strategy::{Regime, Strategy};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Canonical execution order for the CLI's `--strategy all` selection.
pub const BUILTINS: [&str; 5] = [
    "DCA",
    "DipDCA",
    "VolAdjustedDCA",
    "MomentumSMA",
    "RegimeSwitch",
];

pub fn builtin_strategy(
    name: &str,
    interval: u32,
    amount: f64,
    vol_base: f64,
    dip_lookback: usize,
    dip_percent: f64,
) -> Result<Strategy> {
    Ok(match name.to_ascii_uppercase().as_str() {
        "DIPDCA" => Strategy::DipDCA {
            daily_allocation: amount,
            lookback_hours: dip_lookback,
            drop_fraction: dip_percent / 100.0,
        },
        "DCA" => Strategy::DCA {
            interval_hours: interval,
            amount,
        },
        "MOMENTUMSMA" => Strategy::MomentumSMA {
            lookback: 20,
            buy_threshold: 0.01,
            sell_threshold: 0.01,
        },
        "VOLDCA" | "VOLADJUSTEDDCA" => Strategy::VolAdjustedDCA {
            base_amount: vol_base,
            vol_lookback: 30,
            vol_multiplier_max: 2.0,
        },
        "REGIMESWITCH" => Strategy::RegimeSwitch {
            fast_sma: 10,
            slow_sma: 50,
            buy_regime: Regime::Bull,
            sell_regime: Regime::Bear,
        },
        _ => anyhow::bail!("Unknown strategy: {}", name),
    })
}

/// Resolved simulation inputs. The caller supplies the matching price series.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub version: u32,
    pub strategies: Vec<Strategy>,
    pub config: BacktestConfig,
    pub budget: BuyBudget,
    pub include_benchmarks: bool,
}

impl RunSpec {
    pub const VERSION: u32 = 1;

    /// Execute strategies on independent accounts. Emit ledger events only for selected strategies.
    pub fn execute(
        &self,
        series: &PriceSeries,
        mut record: impl FnMut(&str, OrderEvent) -> Result<()>,
    ) -> Result<Vec<BacktestResult>> {
        ensure!(
            self.version == Self::VERSION,
            "Unsupported RunSpec version {}",
            self.version
        );
        ensure!(!self.strategies.is_empty(), "Select at least one strategy");
        let bt = Backtester::new(self.config.clone());
        let mut results = Vec::new();
        for strategy in &self.strategies {
            let result = bt.run_with_budget(series, strategy, &self.budget, |event| {
                record(strategy.name(), event)
            })?;
            results.push(result);
        }
        if self.include_benchmarks {
            let benchmark_budget = BuyBudget {
                min_notional: self.budget.min_notional,
                ..BuyBudget::default()
            };
            let cash = Strategy::Custom {
                name: "Cash".into(),
                config: serde_json::Value::Null,
            };
            results.push(bt.run(series, &cash)?);
            let capital = self.config.initial_capital;
            let fee = self.config.trading_fee;
            let affordable = capital / (1.0 + fee);
            let amount = if affordable + affordable * fee > capital {
                affordable.next_down()
            } else {
                affordable
            };
            let lump_sum = Strategy::DCA {
                interval_hours: u32::MAX,
                amount,
            };
            let mut hold = bt.run_with_budget(series, &lump_sum, &benchmark_budget, |_| Ok(()))?;
            hold.strategy_name = "Buy and hold".into();
            results.push(hold);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Bar;
    use chrono::{TimeZone, Utc};

    #[test]
    fn replay_resolved_spec() -> Result<()> {
        let spec = RunSpec {
            version: RunSpec::VERSION,
            strategies: vec![builtin_strategy("dca", 24, 10.0, 0.0, 168, 5.0)?],
            config: BacktestConfig::default(),
            budget: BuyBudget::default(),
            include_benchmarks: true,
        };
        let series = PriceSeries {
            symbol: "TEST".into(),
            bars: (0..3)
                .map(|hour| Bar {
                    timestamp: Utc.timestamp_opt(hour * 3600, 0).unwrap(),
                    open: 100.0,
                    high: 100.0,
                    low: 100.0,
                    close: 100.0,
                    volume: 1.0,
                })
                .collect(),
        };
        let mut replay: RunSpec = serde_json::from_str(&serde_json::to_string(&spec)?)?;
        let run = |spec: &RunSpec| {
            let mut ledger = Vec::new();
            let results = spec.execute(&series, |name, event| {
                ledger.push((name.to_owned(), event.execution_time, event.quantity));
                Ok(())
            })?;
            Ok::<_, anyhow::Error>((serde_json::to_value(results)?, ledger))
        };
        let actual = run(&replay)?;
        assert_eq!(actual, run(&spec)?);
        assert_eq!(actual.0.as_array().unwrap().len(), 3);
        assert_eq!(actual.1.len(), 1);
        replay.version += 1;
        assert!(run(&replay).is_err());
        Ok(())
    }
}

use anyhow::Result;
use argh::FromArgs;
use std::path::PathBuf;

/// QBacktest - High-performance quantitative backtesting engine
#[derive(FromArgs)]
pub struct Cli {
    /// subcommand
    #[argh(subcommand)]
    pub command: Command,
}

#[argh(subcommand)]
pub enum Command {
    /// Run a backtest on a price series
    Run(RunArgs),
    /// Walk-forward optimization
    WalkForward(WalkForwardArgs),
    /// Generate HTML report
    Report(ReportArgs),
    /// List available strategies
    Strategies(StrategiesArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
/// Run a backtest on a price series
pub struct RunArgs {
    /// input CSV file (OHLCV)
    #[argh(option)]
    pub input: PathBuf,
    /// output report path
    #[argh(option, default = r#"std::path::PathBuf::from("report.html")"#)]
    pub output: PathBuf,
    /// strategy name
    #[argh(option, default = r#""DCA".to_string()"#)]
    pub strategy: String,
    /// DCA interval in hours
    #[argh(option, default = r#"24"#)]
    pub dca_interval: u32,
    /// DCA amount
    #[argh(option, default = r#"100.0"#)]
    pub dca_amount: f64,
    /// initial capital
    #[argh(option, default = r#"10000.0"#)]
    pub capital: f64,
    /// trading fee
    #[argh(option, default = r#"0.001"#)]
    pub fee: f64,
    /// slippage
    #[argh(option, default = r#"0.0005"#)]
    pub slippage: f64,
    /// risk-free rate (annual)
    #[argh(option, default = r#"0.03"#)]
    pub risk_free: f64,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "walk-forward")]
/// Walk-forward optimization
pub struct WalkForwardArgs {
    /// input CSV file
    #[argh(option)]
    pub input: PathBuf,
    /// strategy name
    #[argh(option, default = r#""MomentumSMA".to_string()"#)]
    pub strategy: String,
    /// train months
    #[argh(option, default = r#"12"#)]
    pub train: u32,
    /// test months
    #[argh(option, default = r#"6"#)]
    pub test: u32,
    /// step months
    #[argh(option, default = r#"6"#)]
    pub step: u32,
    /// output path
    #[argh(
        option,
        default = r#"std::path::PathBuf::from("walk_forward_report.html")"#
    )]
    pub output: PathBuf,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "report")]
/// Generate an HTML report from a JSON results file
pub struct ReportArgs {
    /// input JSON file with results
    #[argh(option)]
    pub input: PathBuf,
    /// output HTML path
    #[argh(option, default = r#"std::path::PathBuf::from("report.html")"#)]
    pub output: PathBuf,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "strategies")]
/// List available strategies
pub struct StrategiesArgs {
    /// show detailed strategy parameters
    #[argh(switch)]
    pub verbose: bool,
}

impl RunArgs {
    pub fn execute(&self) -> Result<()> {
        use crate::backtest::{BacktestConfig, Backtester};
        use crate::data::PriceSeries;
        use crate::report::HtmlReport;
        use crate::strategy::Strategy;

        // CF-017: Validate positive intervals and parameters at CLI boundary
        if self.dca_interval == 0 {
            anyhow::bail!("DCA interval must be positive, got 0");
        }
        if self.capital <= 0.0 {
            anyhow::bail!("Initial capital must be positive, got {}", self.capital);
        }
        if self.fee < 0.0 || self.fee >= 1.0 {
            anyhow::bail!("Trading fee must be in [0, 1), got {}", self.fee);
        }
        if self.slippage < 0.0 || self.slippage >= 1.0 {
            anyhow::bail!("Slippage must be in [0, 1), got {}", self.slippage);
        }

        let series = PriceSeries::from_csv(&self.input, "DATA".to_string())?;

        let strategy: Strategy = match self.strategy.to_uppercase().as_str() {
            "DCA" => Strategy::DCA {
                interval_hours: self.dca_interval,
                amount: self.dca_amount,
            },
            "MOMENTUMSMA" => Strategy::MomentumSMA {
                lookback: 20,
                buy_threshold: 0.01,
                sell_threshold: 0.01,
            },
            "VOLDCA" => Strategy::VolAdjustedDCA {
                base_amount: self.dca_amount,
                vol_lookback: 30,
                vol_multiplier_max: 2.0,
            },
            _ => {
                anyhow::bail!("Unknown strategy: {}", self.strategy)
            }
        };

        let config = BacktestConfig {
            initial_capital: self.capital,
            trading_fee: self.fee,
            slippage: self.slippage,
            risk_free_rate: self.risk_free,
        };
        let bt = Backtester::new(config);
        let result = bt.run(&series, &strategy)?;

        let results = vec![result];
        HtmlReport::generate(&results, &self.output)?;

        println!("Report written to {}", self.output.display());
        Ok(())
    }
}

impl WalkForwardArgs {
    pub fn execute(&self) -> Result<()> {
        use crate::backtest::{BacktestConfig, Backtester, WalkForwardConfig};
        use crate::data::PriceSeries;
        use crate::report::HtmlReport;
        use crate::strategy::Strategy;

        // CF-017: Validate positive intervals and window sizes at CLI boundary
        if self.train == 0 {
            anyhow::bail!("Train months must be positive, got 0");
        }
        if self.test == 0 {
            anyhow::bail!("Test months must be positive, got 0");
        }
        if self.step == 0 {
            anyhow::bail!("Step months must be positive, got 0");
        }

        // Parse and validate strategy (CF-016: walk-forward must respect --strategy)
        let strategy_name = self.strategy.to_uppercase();

        // Validate strategy name early with clear error
        match strategy_name.as_str() {
            "DCA" | "MOMENTUMSMA" | "VOLDCA" | "REGIMESWITCH" => {}
            _ => anyhow::bail!(
                "Unknown strategy for walk-forward: {}. Supported: DCA, MomentumSMA, VolAdjustedDCA, RegimeSwitch",
                self.strategy
            ),
        }

        let strategy_factory = move |_train_end: u32, _test_end: u32| -> Strategy {
            match strategy_name.as_str() {
                "DCA" => Strategy::DCA {
                    interval_hours: 24,
                    amount: 100.0,
                },
                "MOMENTUMSMA" => Strategy::MomentumSMA {
                    lookback: 20,
                    buy_threshold: 0.01,
                    sell_threshold: 0.01,
                },
                "VOLDCA" => Strategy::VolAdjustedDCA {
                    base_amount: 100.0,
                    vol_lookback: 30,
                    vol_multiplier_max: 2.0,
                },
                "REGIMESWITCH" => Strategy::RegimeSwitch {
                    fast_sma: 10,
                    slow_sma: 50,
                    buy_regime: crate::strategy::Regime::Bull,
                    sell_regime: crate::strategy::Regime::Bear,
                },
                _ => {
                    // This should not be reached since we validate above, but keep for safety
                    Strategy::MomentumSMA {
                        lookback: 20,
                        buy_threshold: 0.01,
                        sell_threshold: 0.01,
                    }
                }
            }
        };

        let series = PriceSeries::from_csv(&self.input, "DATA".to_string())?;
        let bt = Backtester::new(BacktestConfig::default());

        let wf_config = WalkForwardConfig {
            train_months: self.train,
            test_months: self.test,
            step_months: self.step,
        };

        let wf_result = bt.walk_forward(&series, strategy_factory, &wf_config)?;

        let results = wf_result.out_of_sample_results;
        HtmlReport::generate(&results, &self.output)?;

        println!(
            "Walk-forward complete: {} OOS periods, avg return {:.2}%, consistency {:.0}%",
            results.len(),
            wf_result.total_oos_return * 100.0,
            wf_result.consistency * 100.0
        );
        Ok(())
    }
}

impl ReportArgs {
    pub fn execute(&self) -> Result<()> {
        use crate::backtest::BacktestResult;
        use crate::report::HtmlReport;

        let json = std::fs::read_to_string(&self.input)?;
        let results: Vec<BacktestResult> = serde_json::from_str(&json)?;
        HtmlReport::generate(&results, &self.output)?;
        println!("Report written to {}", self.output.display());
        Ok(())
    }
}

impl StrategiesArgs {
    pub fn execute(&self) -> Result<()> {
        let strategies = vec![
            ("DCA", "Dollar-cost averaging at fixed intervals"),
            ("MomentumSMA", "Buy when price > SMA, sell when price < SMA"),
            (
                "VolAdjustedDCA",
                "Increase DCA amount when volatility is high",
            ),
            (
                "RegimeSwitch",
                "Switch positions based on market regime detection",
            ),
            (
                "Custom",
                "User-defined strategy (requires custom implementation)",
            ),
        ];

        for (name, desc) in strategies {
            println!("• {} - {}", name, desc);
            if self.verbose {
                match name {
                    "DCA" => println!("    Parameters: interval_hours, amount"),
                    "MomentumSMA" => {
                        println!("    Parameters: lookback, buy_threshold, sell_threshold")
                    }
                    "VolAdjustedDCA" => {
                        println!("    Parameters: base_amount, vol_lookback, vol_multiplier_max")
                    }
                    "RegimeSwitch" => {
                        println!("    Parameters: fast_sma, slow_sma, buy_regime, sell_regime")
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

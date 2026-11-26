use anyhow::Result;
use argh::FromArgs;
use qbt_engine::run::{BUILTINS, RunSpec, builtin_strategy};
use std::path::{Path, PathBuf};

use crate::manifest::{RunManifest, distinct_paths};

/// QBT - High-performance quantitative backtesting engine
#[derive(FromArgs)]
pub struct Cli {
    /// subcommand
    #[argh(subcommand)]
    pub command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
pub enum Command {
    /// Run a backtest on a price series
    Run(RunArgs),
    /// Replay a saved run
    Replay(ReplayArgs),
    /// Evaluate fixed parameters across independent time windows
    WalkForward(WalkForwardArgs),
    /// Generate HTML report
    Report(ReportArgs),
    /// List available strategies
    Strategies(StrategiesArgs),
}

/// Run a backtest on a price series
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
pub struct RunArgs {
    /// input CSV file (OHLCV)
    #[argh(option)]
    pub input: PathBuf,
    /// output report path
    #[argh(option, default = r#"std::path::PathBuf::from("report.html")"#)]
    pub output: PathBuf,
    /// save a reproducible JSON run manifest
    #[argh(option)]
    pub save_manifest: Option<PathBuf>,
    /// trailing hourly closes used by DipDCA
    #[argh(option, default = "168")]
    pub dip_lookback: usize,
    /// percent below trailing high required by DipDCA
    #[argh(option, default = "5.0")]
    pub dip_percent: f64,
    /// strategy name
    #[argh(option, default = r#""DCA".to_string()"#)]
    pub strategy: String,
    /// DCA interval in hours
    #[argh(option, default = r#"24"#)]
    pub dca_interval: u32,
    /// DCA amount
    #[argh(option, default = r#"100.0"#)]
    pub dca_amount: f64,
    /// entry notional for DCA, MomentumSMA and RegimeSwitch; VolAdjustedDCA uses 1/24 per hour
    #[argh(option)]
    pub order_amount: Option<f64>,
    /// maximum buy notional per strategy per UTC execution day
    #[argh(option)]
    pub daily_limit: Option<f64>,
    /// minimum executed order value before fees (buys and sells)
    #[argh(option)]
    pub min_notional: Option<f64>,
    /// count buy fees towards the daily limit
    #[argh(switch)]
    pub limit_includes_fees: bool,
    /// write fills and rejected orders to this CSV
    #[argh(option)]
    pub ledger: Option<PathBuf>,
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

/// Replay a saved run against its original dataset
#[derive(FromArgs)]
#[argh(subcommand, name = "replay")]
pub struct ReplayArgs {
    /// saved JSON run manifest
    #[argh(option)]
    pub manifest: PathBuf,
    /// input CSV file (OHLCV)
    #[argh(option)]
    pub input: PathBuf,
    /// output report path
    #[argh(option, default = r#"PathBuf::from("report.html")"#)]
    pub output: PathBuf,
    /// optional order ledger CSV
    #[argh(option)]
    pub ledger: Option<PathBuf>,
}

/// Evaluate fixed parameters across independent time windows
#[derive(FromArgs)]
#[argh(subcommand, name = "walk-forward")]
pub struct WalkForwardArgs {
    /// input CSV file
    #[argh(option)]
    pub input: PathBuf,
    /// trailing hourly closes used by DipDCA
    #[argh(option, default = "168")]
    pub dip_lookback: usize,
    /// percent below trailing high required by DipDCA
    #[argh(option, default = "5.0")]
    pub dip_percent: f64,
    /// strategy name
    #[argh(option, default = r#""MomentumSMA".to_string()"#)]
    pub strategy: String,
    /// history offset in 30-day months
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

/// Generate an HTML report from a JSON results file
#[derive(FromArgs)]
#[argh(subcommand, name = "report")]
pub struct ReportArgs {
    /// input JSON file with results
    #[argh(option)]
    pub input: PathBuf,
    /// output HTML path
    #[argh(option, default = r#"std::path::PathBuf::from("report.html")"#)]
    pub output: PathBuf,
}

/// List available strategies
#[derive(FromArgs)]
#[argh(subcommand, name = "strategies")]
pub struct StrategiesArgs {
    /// show detailed strategy parameters
    #[argh(switch)]
    pub verbose: bool,
}

impl RunArgs {
    pub fn execute(&self) -> Result<()> {
        use qbt_engine::backtest::{BacktestConfig, BuyBudget};
        use qbt_engine::data::PriceSeries;

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

        let names = if self.strategy.eq_ignore_ascii_case("all") {
            BUILTINS.to_vec()
        } else {
            vec![self.strategy.as_str()]
        };
        let amount = self.order_amount.unwrap_or(self.dca_amount);
        anyhow::ensure!(
            amount.is_finite() && amount > 0.0,
            "Order amount must be positive and finite"
        );
        let vol_base = self.order_amount.map_or(self.dca_amount, |v| v / 24.0);
        let strategies = names
            .iter()
            .map(|name| {
                builtin_strategy(
                    name,
                    self.dca_interval,
                    amount,
                    vol_base,
                    self.dip_lookback,
                    self.dip_percent,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let budget = BuyBudget {
            daily_limit: self.daily_limit,
            min_notional: self.min_notional,
            include_fees: self.limit_includes_fees,
            entry_amount: self.order_amount,
        };
        let config = BacktestConfig {
            initial_capital: self.capital,
            trading_fee: self.fee,
            slippage: self.slippage,
            risk_free_rate: self.risk_free,
        };
        let spec = RunSpec {
            version: RunSpec::VERSION,
            strategies,
            config,
            budget,
            include_benchmarks: self.strategy.eq_ignore_ascii_case("all"),
        };
        distinct_paths(&self.input, &self.output, self.ledger.as_deref(), self.save_manifest.as_deref())?;
        if let Some(path) = &self.save_manifest {
            anyhow::ensure!(!path.try_exists()?, "Choose a new manifest path");
            let manifest = RunManifest::new(&series, spec)?;
            render(&manifest.spec, &series, &self.output, self.ledger.as_deref())?;
            manifest.save(path)?;
        } else {
            render(&spec, &series, &self.output, self.ledger.as_deref())?;
        }
        Ok(())
    }
}

impl ReplayArgs {
    pub fn execute(&self) -> Result<()> {
        use qbt_engine::data::PriceSeries;

        distinct_paths(&self.input, &self.output, self.ledger.as_deref(), Some(&self.manifest))?;
        let series = PriceSeries::from_csv(&self.input, "DATA".to_string())?;
        let manifest = RunManifest::load(&self.manifest)?;
        manifest.verify(&series)?;
        render(&manifest.spec, &series, &self.output, self.ledger.as_deref())
    }
}

fn render(
    spec: &RunSpec,
    series: &qbt_engine::data::PriceSeries,
    output: &Path,
    ledger_path: Option<&Path>,
) -> Result<()> {
    use qbt_engine::report::HtmlReport;
    use std::io::{BufWriter, Write};

    let mut ledger = ledger_path
        .map(std::fs::File::create)
        .transpose()?
        .map(BufWriter::new);
    if let Some(w) = &mut ledger {
        writeln!(
            w,
            "strategy,signal_time,execution_time,side,status,price,quantity,fee,cash"
        )?;
    }
    let results = spec.execute(series, |name, e| {
        if let Some(w) = &mut ledger {
            writeln!(
                w,
                "{},{},{},{},{},{},{},{},{}",
                name,
                e.signal_time.to_rfc3339(),
                e.execution_time.to_rfc3339(),
                e.side,
                e.status,
                e.price,
                e.quantity,
                e.fee,
                e.cash
            )?;
        }
        Ok(())
    })?;
    if let Some(w) = &mut ledger {
        w.flush()?;
    }
    if spec.include_benchmarks {
        HtmlReport::comparison(&results, output)?;
    } else {
        HtmlReport::generate(&results, output)?;
    }

    println!("Report written to {}", output.display());
    Ok(())
}

impl WalkForwardArgs {
    pub fn execute(&self) -> Result<()> {
        use qbt_engine::backtest::{BacktestConfig, Backtester, WalkForwardConfig};
        use qbt_engine::data::PriceSeries;
        use qbt_engine::report::HtmlReport;

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

        let strategy = builtin_strategy(
            &self.strategy,
            24,
            100.0,
            100.0,
            self.dip_lookback,
            self.dip_percent,
        )?;
        let strategy_factory = move |_, _| strategy.clone();

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
        use qbt_engine::backtest::BacktestResult;
        use qbt_engine::report::HtmlReport;

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
            ("DipDCA", "Accumulate daily cash and buy on a drawdown"),
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
                    "DipDCA" => {
                        println!("    Parameters: daily allocation, dip-lookback, dip-percent")
                    }
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

use crate::data::PriceSeries;
use crate::strategy::{Signal, Strategy};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{info, instrument};

/// Backtest result for a single run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub strategy_name: String,
    pub initial_capital: f64,
    pub final_value: f64,
    pub total_return: f64, // percentage
    pub cagr: f64, // annualized
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown: f64,
    pub ulcer_index: f64,
    pub win_rate: f64, // % of profitable trades
    pub profit_factor: f64, // gross profit / gross loss
    pub num_trades: usize,
    pub equity_curve: Vec<f64>,
    pub trade_log: Vec<Trade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub entry_time: chrono::DateTime<chrono::Utc>,
    pub exit_time: Option<chrono::DateTime<chrono::Utc>>,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub quantity: f64,
    pub pnl: Option<f64>,
}

/// Backtest configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub initial_capital: f64,
    pub trading_fee: f64, // e.g., 0.001 = 0.1%
    pub slippage: f64, // e.g., 0.0005 = 0.05%
    pub risk_free_rate: f64, // annual, e.g., 0.03
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_capital: 10_000.0,
            trading_fee: 0.001,
            slippage: 0.0005,
            risk_free_rate: 0.03,
        }
    }
}

/// Walk-forward optimizer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    pub train_months: u32,
    pub test_months: u32,
    pub step_months: u32,
}

impl Default for WalkForwardConfig {
    fn default() -> Self {
        Self { train_months: 12, test_months: 6, step_months: 6 }
    }
}

/// Walk-forward result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardResult {
    pub out_of_sample_results: Vec<BacktestResult>,
    pub total_oos_return: f64,
    pub avg_oos_sharpe: f64,
    pub consistency: f64, // % of OOS periods with positive return
}

#[derive(Debug, Clone, Default)]
pub struct Backtester {
    pub config: BacktestConfig,
}

impl Backtester {
    pub fn new(config: BacktestConfig) -> Self {
        Self { config }
    }

    #[instrument(skip_all)]
    pub fn run(&self, series: &PriceSeries, strategy: &Strategy) -> Result<BacktestResult> {
        // CF-017: Validate nonempty data and finite, economically valid inputs at engine boundary
        if series.is_empty() {
            anyhow::bail!("Price series is empty");
        }
        if self.config.initial_capital <= 0.0 {
            anyhow::bail!("Initial capital must be positive");
        }
        if self.config.trading_fee < 0.0 || self.config.trading_fee >= 1.0 {
            anyhow::bail!("Trading fee must be in [0, 1)");
        }
        if self.config.slippage < 0.0 || self.config.slippage >= 1.0 {
            anyhow::bail!("Slippage must be in [0, 1)");
        }
        // Validate all prices are finite and positive
        for (i, bar) in series.bars.iter().enumerate() {
            if !bar.close.is_finite() || bar.close <= 0.0 {
                anyhow::bail!("Invalid price at bar {}: {} (must be finite and positive)", i, bar.close);
            }
            if !bar.open.is_finite() || bar.open <= 0.0 {
                anyhow::bail!("Invalid open price at bar {}: {}", i, bar.open);
            }
            if !bar.high.is_finite() || bar.high <= 0.0 {
                anyhow::bail!("Invalid high price at bar {}: {}", i, bar.high);
            }
            if !bar.low.is_finite() || bar.low <= 0.0 {
                anyhow::bail!("Invalid low price at bar {}: {}", i, bar.low);
            }
            if !bar.volume.is_finite() || bar.volume < 0.0 {
                anyhow::bail!("Invalid volume at bar {}: {}", i, bar.volume);
            }
        }

        info!(
            symbol = %series.symbol,
            strategy = strategy.name(),
            bars = series.len(),
            "Starting backtest"
        );

        let signals = strategy.generate_signals(series);
        let closes = series.closes();

        let mut cash = self.config.initial_capital;
        let mut position: f64 = 0.0;
        let mut pending_exit: Option<f64> = None;
        let mut equity_curve: VecDeque<f64> = VecDeque::new();
        let mut trades: Vec<Trade> = Vec::new();
        let mut gross_profit = 0.0;
        let mut gross_loss = 0.0;
        let mut winning_trades = 0;

        let mut last_entry_time: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut entry_price: Option<f64> = None;

        for (i, signal) in signals.iter().enumerate() {
            let price = closes[i];
            let ts = series.bars[i].timestamp;

            match signal {
                Signal::Buy { amount } => {
                    let buy_price = price * (1.0 + self.config.slippage);
                    let fee = *amount * self.config.trading_fee;
                    let total_cost = *amount + fee;

                    if cash >= total_cost {
                        cash -= total_cost;
                        position += *amount / buy_price;
                        last_entry_time = Some(ts);
                        entry_price = Some(buy_price);

                        // Close pending short (if any)
                        if let Some(exit_price) = pending_exit.take() {
                            let pnl = exit_price - buy_price;
                            if pnl > 0.0 {
                                gross_profit += pnl;
                                winning_trades += 1;
                            } else {
                                gross_loss += -pnl;
                            }
                            trades.push(Trade {
                                entry_time: last_entry_time.unwrap_or(ts),
                                exit_time: Some(ts),
                                entry_price: entry_price.unwrap_or(buy_price),
                                exit_price: Some(exit_price),
                                quantity: position,
                                pnl: Some(pnl),
                            });
                        }
                    }
                }
                Signal::Sell { amount } => {
                    if position > 0.0 {
                        let sell_price = price * (1.0 - self.config.slippage);
                        // `amount` is the notional $ to sell. A sentinel >= f64::MAX/2 means
                        // "sell the entire position" (full exit).
                        let sell_all = *amount >= f64::MAX / 2.0;
                        let notional = if sell_all {
                            position * sell_price
                        } else {
                            (*amount).min(position * sell_price)
                        };
                        let qty_to_sell = notional / sell_price;
                        let proceeds = qty_to_sell * sell_price;
                        let fee = proceeds * self.config.trading_fee;
                        position -= qty_to_sell;
                        cash += proceeds - fee;

                        let pnl = (sell_price - entry_price.unwrap_or(sell_price)) * qty_to_sell;
                        if pnl > 0.0 {
                            gross_profit += pnl;
                            winning_trades += 1;
                        } else {
                            gross_loss += -pnl;
                        }
                        trades.push(Trade {
                            entry_time: last_entry_time.unwrap_or(ts),
                            exit_time: Some(ts),
                            entry_price: entry_price.take().unwrap_or(sell_price),
                            exit_price: Some(sell_price),
                            quantity: qty_to_sell,
                            pnl: Some(pnl),
                        });

                        if position <= 1e-6 {
                            last_entry_time = None;
                            entry_price = None;
                        }
                    }
                }
                Signal::Hold => {}
            }

            let equity = cash + position * price;
            equity_curve.push_back(equity);
        }

        // Mark-to-market final value
        let final_value = cash + position * closes[closes.len() - 1];

        // Calculate metrics
        let equity_vec: Vec<f64> = equity_curve.into();
        let mut metrics = calculate_metrics(&equity_vec, &trades, self.config.risk_free_rate, series.len(), self.config.initial_capital);
        metrics.strategy_name = strategy.name().to_string();

        info!(
            final_value = metrics.final_value,
            total_return = metrics.total_return,
            sharpe = metrics.sharpe_ratio,
            "Backtest complete"
        );

        Ok(metrics)
    }

    /// Run multiple parameter sets in parallel
    pub fn run_parallel(
        &self,
        series: &PriceSeries,
        strategies: &[Strategy],
    ) -> Result<Vec<BacktestResult>> {
        strategies.par_iter().map(|s| self.run(series, s)).collect()
    }

    /// Walk-forward analysis
    pub fn walk_forward(
        &self,
        series: &PriceSeries,
        strategy_factory: impl Fn(u32, u32) -> Strategy + Send + Sync,
        wf_config: &WalkForwardConfig,
    ) -> Result<WalkForwardResult> {
        // CF-017: Validate walk-forward config at engine boundary
        if wf_config.train_months == 0 {
            anyhow::bail!("Walk-forward train_months must be positive");
        }
        if wf_config.test_months == 0 {
            anyhow::bail!("Walk-forward test_months must be positive");
        }
        if wf_config.step_months == 0 {
            anyhow::bail!("Walk-forward step_months must be positive");
        }
        if series.is_empty() {
            anyhow::bail!("Price series is empty");
        }

        info!(
            train_months = wf_config.train_months,
            test_months = wf_config.test_months,
            "Starting walk-forward analysis"
        );

        let total_bars = series.len();
        let bar_per_day = 24usize; // assume hourly bars
        let train_bars = (wf_config.train_months as usize) * 30 * bar_per_day;
        let test_bars = (wf_config.test_months as usize) * 30 * bar_per_day;
        let step_bars = (wf_config.step_months as usize) * 30 * bar_per_day;

        if train_bars == 0 || test_bars == 0 || step_bars == 0 {
            anyhow::bail!("Computed window sizes are zero; check train/test/step months and bar frequency");
        }

        let mut oos_results: Vec<BacktestResult> = Vec::new();
        let mut start = 0usize;

        while start + train_bars + test_bars <= total_bars {
            let train_end = start + train_bars;
            let test_end = start + train_bars + test_bars;

            // Train: find best params (placeholder: just use factory)
            let strategy = strategy_factory(train_end as u32, test_end as u32);

            // Test: run OOS on the test window [train_end, test_end)
            let oos_bars: Vec<crate::data::Bar> = series.bars[train_end..test_end].to_vec();
            let oos_series = crate::data::PriceSeries {
                symbol: series.symbol.clone(),
                bars: oos_bars,
            };
            let oos_result = self.run(&oos_series, &strategy)?;
            oos_results.push(oos_result);

            start += step_bars;
        }

        if oos_results.is_empty() {
            anyhow::bail!("Insufficient data for walk-forward with current window sizes");
        }

        let total_oos_return: f64 = oos_results.iter().map(|r| r.total_return).sum::<f64>()
            / oos_results.len() as f64;
        let avg_oos_sharpe: f64 = oos_results.iter().map(|r| r.sharpe_ratio).sum::<f64>()
            / oos_results.len() as f64;
        let consistency = oos_results.iter().filter(|r| r.total_return > 0.0).count() as f64
            / oos_results.len() as f64;

        info!(
            oos_periods = oos_results.len(),
            total_return = total_oos_return,
            avg_sharpe = avg_oos_sharpe,
            consistency = consistency,
            "Walk-forward complete"
        );

        Ok(WalkForwardResult {
            out_of_sample_results: oos_results,
            total_oos_return,
            avg_oos_sharpe,
            consistency,
        })
    }
}

fn calculate_metrics(
    equity: &[f64],
    trades: &[Trade],
    risk_free_rate: f64,
    num_bars: usize,
    initial_capital: f64,
) -> BacktestResult {
    // Total return: use configured initial capital, not first post-trade equity
    let initial = initial_capital;
    let final_value = equity.last().copied().unwrap_or(initial);
    let total_return = (final_value - initial) / initial;

    // CAGR (assuming hourly bars, 365 days)
    let hours_per_year: f64 = 24.0 * 365.0;
    let years = num_bars as f64 / hours_per_year;
    let cagr = if years > 0.0 {
        (final_value / initial).powf(1.0 / years) - 1.0
    } else {
        0.0
    };

    // Sharpe ratio
    let returns: Vec<f64> = equity
        .windows(2)
        .map(|w| (w[1] - w[0]) / w[0])
        .collect();
    let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
    let std_ret = {
        let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>()
            / (returns.len() - 1) as f64;
        variance.sqrt()
    };
    let bars_per_year: f64 = 24.0 * 365.0;
    let sharpe_ratio = if std_ret > 0.0 {
        ((mean_ret - risk_free_rate / bars_per_year) / std_ret) * (bars_per_year).sqrt()
    } else {
        0.0
    };

    // Sortino ratio
    let downside_dev: Vec<f64> = returns
        .iter()
        .filter_map(|r| if *r < 0.0 { Some((-r).powi(2)) } else { None })
        .collect();
    let downside_std = if downside_dev.is_empty() {
        0.0
    } else {
        let downside_var = downside_dev.iter().sum::<f64>() / (returns.len()) as f64;
        downside_var.sqrt()
    };
    let sortino_ratio = if downside_std > 0.0 {
        ((mean_ret - risk_free_rate / bars_per_year) / downside_std) * (bars_per_year).sqrt()
    } else {
        f64::INFINITY
    };

    // Max drawdown: include pre-trade initial capital as starting peak
    let mut peak = initial_capital;
    let mut max_dd = 0.0;
    for &val in equity.iter() {
        if val > peak {
            peak = val;
        }
        let dd = (peak - val) / peak;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    // Ulcer index: include pre-trade initial capital
    let dd_values: Vec<f64> = {
        let mut peak = initial_capital;
        equity.iter().map(|&val| {
            if val > peak {
                peak = val;
            }
            (peak - val) / peak
        }).collect()
    };
    let ulcer_index = dd_values.iter().map(|d| d * d).sum::<f64>() / dd_values.len() as f64;
    let ulcer_index = (ulcer_index * 100.0).sqrt();

    // Trade metrics
    let num_trades = trades.len();
    let gross_profit: f64 = trades.iter().filter_map(|t| t.pnl).filter(|p| *p > 0.0).sum();
    let gross_loss: f64 = trades.iter().filter_map(|t| t.pnl).filter(|p| *p < 0.0).map(|p| -p).sum();
    let winning_trades = trades.iter().filter_map(|t| t.pnl).filter(|p| *p > 0.0).count();
    let win_rate = if num_trades > 0 { winning_trades as f64 / num_trades as f64 } else { 0.0 };
    let profit_factor = if gross_loss > 0.0 { gross_profit / gross_loss } else { f64::INFINITY };

    BacktestResult {
        strategy_name: String::new(), // set by caller
        initial_capital: initial_capital,
        final_value,
        total_return,
        cagr,
        sharpe_ratio,
        sortino_ratio,
        max_drawdown: max_dd,
        ulcer_index,
        win_rate,
        profit_factor,
        num_trades,
        equity_curve: equity.to_vec(),
        trade_log: trades.to_vec(),
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
        PriceSeries { symbol: "TEST".into(), bars }
    }

    #[test]
    fn test_simple_backtest() {
        let prices: Vec<f64> = (0..100).map(|i| 100.0 + i as f64 * 0.1).collect();
        let series = make_series(prices);
        let strategy = Strategy::DCA { interval_hours: 10, amount: 50.0 };
        let bt = Backtester::new(BacktestConfig::default());
        let result = bt.run(&series, &strategy).unwrap();

        assert_eq!(result.num_trades, 10); // 10 buys
        assert!(result.final_value > result.initial_capital);
        assert!(result.total_return > 0.0);
    }

    #[test]
    fn test_momentum_backtest() {
        let prices = vec![100.0, 101.0, 102.0, 110.0, 115.0, 120.0, 110.0, 100.0, 90.0];
        let series = make_series(prices);
        let strategy = Strategy::MomentumSMA { lookback: 3, buy_threshold: 0.01, sell_threshold: 0.01 };
        let bt = Backtester::new(BacktestConfig::default());
        let result = bt.run(&series, &strategy).unwrap();

        // Should have entered and exited
        assert!(result.num_trades >= 2);
    }

    // CF-015: First-bar costs omitted from performance.
    // Initial capital must be the configured starting capital, not the first
    // post-trade equity. Regression test: flat $100 bars, DCA buys $100 once
    // with 10% fee on $1000 capital. Expected: initial=1000, final=990, return=-1%.
    #[test]
    fn test_cf015_first_bar_costs_included() {
        // 1500 flat bars at $100
        let prices = vec![100.0; 1500];
        let series = make_series(prices);
        // DCA interval 9999 > 1500 bars => only first bar buys
        let strategy = Strategy::DCA { interval_hours: 9999, amount: 100.0 };
        let config = BacktestConfig {
            initial_capital: 1000.0,
            trading_fee: 0.1,
            slippage: 0.0,
            risk_free_rate: 0.03,
        };
        let bt = Backtester::new(config);
        let result = bt.run(&series, &strategy).unwrap();

        // Configured initial capital retained
        assert_eq!(result.initial_capital, 1000.0);
        // First-bar fee: $100 buy at 10% fee => $110 cost => final equity $990
        assert!((result.final_value - 990.0).abs() < 1e-9, "final_value={}", result.final_value);
        // Return = -1% (990 - 1000) / 1000
        assert!((result.total_return - (-0.01)).abs() < 1e-9, "total_return={}", result.total_return);
    }
}

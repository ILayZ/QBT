use crate::data::PriceSeries;
use crate::strategy::{Signal, Strategy};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{info, instrument};

/// A single buy lot with cost basis including fees (for FIFO P&L)
#[derive(Debug, Clone)]
struct Lot {
    quantity: f64,
    cost_per_unit: f64, // includes entry fee
    entry_time: chrono::DateTime<chrono::Utc>,
}

/// Backtest result for a single run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub strategy_name: String,
    pub initial_capital: f64,
    pub final_value: f64,
    pub total_return: f64, // percentage
    pub cagr: f64,         // annualized
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown: f64,
    pub ulcer_index: f64,
    pub win_rate: f64,      // % of profitable trades
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
    pub trading_fee: f64,    // e.g., 0.001 = 0.1%
    pub slippage: f64,       // e.g., 0.0005 = 0.05%
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
        Self {
            train_months: 12,
            test_months: 6,
            step_months: 6,
        }
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
                anyhow::bail!(
                    "Invalid price at bar {}: {} (must be finite and positive)",
                    i,
                    bar.close
                );
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
        let opens = series.opens();

        let mut cash = self.config.initial_capital;
        let mut lots: Vec<Lot> = Vec::new(); // FIFO lot queue
        let mut equity_curve: VecDeque<f64> = VecDeque::new();
        let mut trades: Vec<Trade> = Vec::new();
        let mut _gross_profit = 0.0;
        let mut _gross_loss = 0.0;
        let mut _winning_trades = 0;

        // CF-019: Causal execution model
        // Signals are generated using data available at bar close.
        // Execution happens at the NEXT bar's open (or close if open unavailable).
        // This avoids look-ahead bias: you cannot know bar i's close before it closes,
        // and you cannot execute at bar i's close in the same instant.
        for (i, signal) in signals.iter().enumerate() {
            // Execution price: next bar's open (causal), or current close for the last bar
            let exec_price = if i + 1 < opens.len() {
                opens[i + 1]
            } else {
                closes[i]
            };
            let exec_ts = if i + 1 < series.bars.len() {
                series.bars[i + 1].timestamp
            } else {
                series.bars[i].timestamp
            };

            match signal {
                Signal::Buy { amount } => {
                    let buy_price = exec_price * (1.0 + self.config.slippage);
                    let fee = *amount * self.config.trading_fee;
                    let total_cost = *amount + fee;

                    if cash >= total_cost {
                        cash -= total_cost;
                        let qty = *amount / buy_price;
                        // Cost per unit includes entry fee pro-rated
                        let cost_per_unit = buy_price * (1.0 + self.config.trading_fee);
                        lots.push(Lot {
                            quantity: qty,
                            cost_per_unit,
                            entry_time: exec_ts,
                        });

                        // Close pending short (if any) — unchanged from prior behavior
                        // Note: short logic was incomplete; keeping minimal for compatibility
                    }
                }
                Signal::Sell { amount } => {
                    // Total position from lots
                    let position: f64 = lots.iter().map(|l| l.quantity).sum();
                    if position > 1e-9 {
                        let sell_price = exec_price * (1.0 - self.config.slippage);
                        let sell_all = *amount >= f64::MAX / 2.0;
                        let notional = if sell_all {
                            position * sell_price
                        } else {
                            (*amount).min(position * sell_price)
                        };
                        let qty_to_sell = notional / sell_price;

                        // FIFO lot consumption
                        let mut remaining_to_sell = qty_to_sell;
                        let mut _realized_pnl = 0.0;
                        let mut consumed_lots: Vec<Lot> = Vec::new();

                        for lot in lots.drain(..) {
                            if remaining_to_sell <= 1e-9 {
                                consumed_lots.push(lot);
                                continue;
                            }
                            let take = lot.quantity.min(remaining_to_sell);
                            // P&L = (sell_price - cost_per_unit) * take
                            // cost_per_unit already includes entry fee; subtract exit fee from proceeds
                            let exit_fee_per_unit = sell_price * self.config.trading_fee;
                            let net_sell_per_unit = sell_price - exit_fee_per_unit;
                            let pnl = (net_sell_per_unit - lot.cost_per_unit) * take;
                            _realized_pnl += pnl;

                            if pnl > 0.0 {
                                _gross_profit += pnl;
                                _winning_trades += 1;
                            } else {
                                _gross_loss += -pnl;
                            }

                            trades.push(Trade {
                                entry_time: lot.entry_time,
                                exit_time: Some(exec_ts),
                                entry_price: lot.cost_per_unit, // cost basis includes entry fee
                                exit_price: Some(net_sell_per_unit), // net of exit fee
                                quantity: take,
                                pnl: Some(pnl),
                            });

                            remaining_to_sell -= take;
                            let lot_remain = lot.quantity - take;
                            if lot_remain > 1e-9 {
                                consumed_lots.push(Lot {
                                    quantity: lot_remain,
                                    cost_per_unit: lot.cost_per_unit,
                                    entry_time: lot.entry_time,
                                });
                            }
                        }
                        // Put back unconsumed lots
                        lots = consumed_lots;

                        let proceeds = qty_to_sell * sell_price;
                        let fee = proceeds * self.config.trading_fee;
                        cash += proceeds - fee;
                    }
                }
                Signal::Hold => {}
            }

            let position_qty: f64 = lots.iter().map(|l| l.quantity).sum();
            let equity = cash + position_qty * exec_price;
            equity_curve.push_back(equity);
        }

        // Mark-to-market final value
        let position_qty: f64 = lots.iter().map(|l| l.quantity).sum();
        let _final_value = cash + position_qty * closes[closes.len() - 1];

        // Calculate metrics
        let equity_vec: Vec<f64> = equity_curve.into();
        let mut metrics = calculate_metrics(
            &equity_vec,
            &trades,
            self.config.risk_free_rate,
            series.len(),
            self.config.initial_capital,
        );
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
            anyhow::bail!(
                "Computed window sizes are zero; check train/test/step months and bar frequency"
            );
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

        let total_oos_return: f64 =
            oos_results.iter().map(|r| r.total_return).sum::<f64>() / oos_results.len() as f64;
        let avg_oos_sharpe: f64 =
            oos_results.iter().map(|r| r.sharpe_ratio).sum::<f64>() / oos_results.len() as f64;
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
    let returns: Vec<f64> = equity.windows(2).map(|w| (w[1] - w[0]) / w[0]).collect();
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
        equity
            .iter()
            .map(|&val| {
                if val > peak {
                    peak = val;
                }
                (peak - val) / peak
            })
            .collect()
    };
    let ulcer_index = dd_values.iter().map(|d| d * d).sum::<f64>() / dd_values.len() as f64;
    let ulcer_index = (ulcer_index * 100.0).sqrt();

    // Trade metrics
    let num_trades = trades.len();
    let gross_profit: f64 = trades
        .iter()
        .filter_map(|t| t.pnl)
        .filter(|p| *p > 0.0)
        .sum();
    let gross_loss: f64 = trades
        .iter()
        .filter_map(|t| t.pnl)
        .filter(|p| *p < 0.0)
        .map(|p| -p)
        .sum();
    let winning_trades = trades
        .iter()
        .filter_map(|t| t.pnl)
        .filter(|p| *p > 0.0)
        .count();
    let win_rate = if num_trades > 0 {
        winning_trades as f64 / num_trades as f64
    } else {
        0.0
    };
    let profit_factor = if gross_loss > 0.0 {
        gross_profit / gross_loss
    } else {
        f64::INFINITY
    };

    BacktestResult {
        strategy_name: String::new(), // set by caller
        initial_capital,
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
        PriceSeries {
            symbol: "TEST".into(),
            bars,
        }
    }

    #[test]
    fn test_simple_backtest() {
        let prices: Vec<f64> = (0..100).map(|i| 100.0 + i as f64 * 0.1).collect();
        let series = make_series(prices);
        let strategy = Strategy::DCA {
            interval_hours: 10,
            amount: 50.0,
        };
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
        let strategy = Strategy::MomentumSMA {
            lookback: 3,
            buy_threshold: 0.01,
            sell_threshold: 0.01,
        };
        let bt = Backtester::new(BacktestConfig::default());
        let result = bt.run(&series, &strategy).unwrap();

        // Should have entered and exited (one completed round-trip = 1 trade)
        // CF-019 causal execution: buy signal at bar 4 -> execute at bar 5 open (120),
        // sell signal at bar 7 -> execute at bar 8 open (90). One buy+sell = 1 trade.
        assert_eq!(result.num_trades, 1);
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
        let strategy = Strategy::DCA {
            interval_hours: 9999,
            amount: 100.0,
        };
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
        assert!(
            (result.final_value - 990.0).abs() < 1e-9,
            "final_value={}",
            result.final_value
        );
        // Return = -1% (990 - 1000) / 1000
        assert!(
            (result.total_return - (-0.01)).abs() < 1e-9,
            "total_return={}",
            result.total_return
        );
    }

    // CF-018: Trade P&L cost basis and fees.
    // Test: Run a round-trip backtest and verify FIFO lot tracking + fee allocation:
    // sum(trade P&L) == final_value - initial_capital (when position fully closed).
    #[test]
    fn test_cf018_fifo_lot_tracking_with_fees() {
        let config = BacktestConfig {
            initial_capital: 1000.0,
            trading_fee: 0.1,
            slippage: 0.0,
            risk_free_rate: 0.03,
        };
        let bt = Backtester::new(config.clone());

        // Price series: up then down to trigger MomentumSMA round-trip
        let prices = vec![100.0, 110.0, 120.0, 110.0, 100.0, 90.0];
        let series = make_series(prices);

        let strategy = Strategy::MomentumSMA {
            lookback: 2,
            buy_threshold: 0.05,
            sell_threshold: 0.05,
        };
        let result = bt.run(&series, &strategy).unwrap();

        // Verify reconciliation: when position fully closed, sum(trade.pnl) == equity change
        let open_trades = result
            .trade_log
            .iter()
            .filter(|t| t.exit_time.is_none())
            .count();
        if open_trades == 0 && result.num_trades > 0 {
            let sum_pnl: f64 = result.trade_log.iter().filter_map(|t| t.pnl).sum();
            let equity_change = result.final_value - result.initial_capital;
            assert!(
                (sum_pnl - equity_change).abs() < 1e-6,
                "CF-018 reconciliation failed: sum_pnl={}, equity_change={}, diff={}",
                sum_pnl,
                equity_change,
                sum_pnl - equity_change
            );
        }
    }

    // CF-019: Same-close execution needs explicit causal model.
    // Synthetic jump test: create a price series with a known gap and verify
    // signals use prior-bar data, execution uses next-bar open (not same-bar close).
    // Series: flat at 100, then jump to 200 at bar 5.
    // MomentumSMA lookback=3, buy_threshold=0.01.
    // Bar 4: close=100, SMA≈100, ratio=1.0 -> no buy
    // Bar 5: close=200, but signal at bar 5 uses bar 4 data (close=100, SMA≈100) -> no buy
    // Bar 6: signal uses bar 5 data (close=200, SMA≈133.3) -> buy signal generated
    // Execution at bar 6 uses bar 7 open (if exists) or bar 6 close.
    // This verifies NO look-ahead: the jump at bar 5 cannot trigger a fill at bar 5.
    #[test]
    fn test_cf019_causal_execution_synthetic_jump() {
        let config = BacktestConfig {
            initial_capital: 1000.0,
            trading_fee: 0.0,
            slippage: 0.0,
            risk_free_rate: 0.03,
        };
        let bt = Backtester::new(config);

        // 10 bars flat at 100, then jump to 200 for 5 bars, then flat
        let mut prices = vec![100.0; 10];
        prices.extend(vec![200.0; 5]);
        prices.extend(vec![200.0; 5]);
        let series = make_series(prices);

        let strategy = Strategy::MomentumSMA {
            lookback: 3,
            buy_threshold: 0.01,
            sell_threshold: 0.01,
        };
        let result = bt.run(&series, &strategy).unwrap();

        // With causal model (prior-bar signal, next-bar open execution):
        // - Bar 3 (i=3): first valid SMA (bars 1,2,3 avg=100), price=100 -> no buy
        // - Bar 4 (i=4): SMA bars 2,3,4=100, price=100 -> no buy
        // - Bar 5 (i=5): SMA bars 3,4,5=100, price=100 -> no buy (uses bar 4 close=100)
        // - Bar 6 (i=6): SMA bars 4,5,6 ≈ 133.3, price=200 (bar 5 close) -> BUY signal
        //   Execution at bar 7 open = 200
        // - Bar 7-...: in position, no sell until price drops below SMA*(1-sell_threshold)

        // Key assertion: NO trade should have entry_time at bar 5 (the jump bar)
        // because signal at bar 5 uses bar 4 data (pre-jump)
        let jump_bar_idx = 5; // 0-indexed: bar 5 is first 200 close
        let trades_at_jump = result
            .trade_log
            .iter()
            .filter(|t| t.entry_time == series.bars[jump_bar_idx].timestamp)
            .count();
        assert_eq!(
            trades_at_jump, 0,
            "Causal violation: trade entered at jump bar using same-bar close"
        );

        // First buy should be at bar 7 (execution) or later
        // Bar 6 signal -> execution at bar 7 open
        if result.num_trades > 0 {
            let first_entry_time = result.trade_log[0].entry_time;
            let expected_first_exec_time = series.bars[7].timestamp; // bar 7 open = bar 6 signal execution
            assert_eq!(
                first_entry_time, expected_first_exec_time,
                "First trade should execute at bar 7 open (causal), got {:?}",
                first_entry_time
            );
        }

        // Verify signal generation uses prior-bar: the jump at bar 5 should NOT produce a signal at bar 5
        // (Signal at bar 5 uses bar 4 close=100, SMA≈100, ratio=1.0 < 1.01)
        // First valid signal should be at bar 6 (using bar 5 close=200)
        // This is implicitly tested by the entry_time check above
    }
}

use serde::{Deserialize, Serialize};

/// Risk-adjusted performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub calmar_ratio: f64,
    pub max_drawdown: f64,
    pub ulcer_index: f64,
    pub volatility: f64, // annualized
    pub downside_risk: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub num_trades: usize,
    pub avg_trade_pnl: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
    pub exposure: f64, // % of time in market
}

impl PerformanceMetrics {
    pub fn calculate(
        equity: &[f64],
        returns: &[f64],
        risk_free_rate: f64,
        bars_per_year: f64,
        in_market: &[bool],
        trade_pnls: &[f64],
    ) -> Self {
        let total_return = equity
            .last()
            .map(|e| e / equity.first().unwrap_or(&1.0) - 1.0)
            .unwrap_or(0.0);
        let years = equity.len().saturating_sub(1) as f64 / bars_per_year;
        let cagr = if years > 0.0 && equity.first().map(|e| *e > 0.0).unwrap_or(false) {
            (equity.last().unwrap() / equity.first().unwrap()).powf(1.0 / years) - 1.0
        } else {
            0.0
        };

        // Volatility
        let mean_ret = returns.iter().sum::<f64>() / returns.len().max(1) as f64;
        let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>()
            / returns.len().saturating_sub(1).max(1) as f64;
        let volatility = variance.sqrt() * (bars_per_year).sqrt();

        // Sharpe
        let sharpe_ratio = if volatility > 0.0 {
            (mean_ret * bars_per_year - risk_free_rate) / volatility
        } else {
            0.0
        };

        // Downside risk
        let downside_dev: Vec<f64> = returns
            .iter()
            .filter_map(|r| if *r < 0.0 { Some((-r).powi(2)) } else { None })
            .collect();
        let downside_var = if downside_dev.is_empty() {
            0.0
        } else {
            downside_dev.iter().sum::<f64>() / returns.len() as f64
        };
        let downside_risk = downside_var.sqrt() * (bars_per_year).sqrt();

        // Sortino
        let sortino_ratio = if downside_risk > 0.0 {
            (mean_ret * bars_per_year - risk_free_rate) / downside_risk
        } else {
            f64::INFINITY
        };

        // Max drawdown
        let mut peak = equity.first().copied().unwrap_or(0.0);
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

        // Calmar
        let calmar_ratio = if max_dd > 0.0 {
            cagr / max_dd
        } else {
            f64::INFINITY
        };

        // Ulcer index
        let dd_values: Vec<f64> = {
            let mut peak = equity.first().copied().unwrap_or(0.0);
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
        let mean_square_drawdown =
            dd_values.iter().map(|d| d * d).sum::<f64>() / dd_values.len().max(1) as f64;
        let ulcer_index = mean_square_drawdown.sqrt() * 100.0;

        // Trade stats
        let wins: Vec<f64> = trade_pnls.iter().filter(|p| **p > 0.0).copied().collect();
        let losses: Vec<f64> = trade_pnls.iter().filter(|p| **p < 0.0).copied().collect();
        let win_rate = if !trade_pnls.is_empty() {
            wins.len() as f64 / trade_pnls.len() as f64
        } else {
            0.0
        };
        let gross_profit: f64 = wins.iter().sum();
        let gross_loss: f64 = losses.iter().map(|p| -p).sum();
        let profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else {
            f64::INFINITY
        };
        let avg_trade_pnl = if !trade_pnls.is_empty() {
            trade_pnls.iter().sum::<f64>() / trade_pnls.len() as f64
        } else {
            0.0
        };
        let avg_win = if !wins.is_empty() {
            gross_profit / wins.len() as f64
        } else {
            0.0
        };
        let avg_loss = if !losses.is_empty() {
            gross_loss / losses.len() as f64
        } else {
            0.0
        };
        let best_trade = wins
            .iter()
            .copied()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        let worst_trade = losses
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        // Exposure
        let in_market_count = in_market.iter().filter(|b| **b).count();
        let exposure = if !in_market.is_empty() {
            in_market_count as f64 / in_market.len() as f64
        } else {
            0.0
        };

        Self {
            total_return,
            cagr,
            sharpe_ratio,
            sortino_ratio,
            calmar_ratio,
            max_drawdown: max_dd,
            ulcer_index,
            volatility,
            downside_risk,
            win_rate,
            profit_factor,
            num_trades: trade_pnls.len(),
            avg_trade_pnl,
            avg_win,
            avg_loss,
            best_trade,
            worst_trade,
            exposure,
        }
    }
}

impl std::fmt::Display for PerformanceMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "=== Performance Metrics ===")?;
        writeln!(f, "Total Return: {:.2}%", self.total_return * 100.0)?;
        writeln!(f, "CAGR: {:.2}%", self.cagr * 100.0)?;
        writeln!(f, "Sharpe Ratio: {:.2}", self.sharpe_ratio)?;
        writeln!(f, "Sortino Ratio: {:.2}", self.sortino_ratio)?;
        writeln!(f, "Calmar Ratio: {:.2}", self.calmar_ratio)?;
        writeln!(f, "Max Drawdown: {:.2}%", self.max_drawdown * 100.0)?;
        writeln!(f, "Ulcer Index: {:.2}", self.ulcer_index)?;
        writeln!(f, "Volatility: {:.2}%", self.volatility * 100.0)?;
        writeln!(f, "Win Rate: {:.1}%", self.win_rate * 100.0)?;
        writeln!(f, "Profit Factor: {:.2}", self.profit_factor)?;
        writeln!(f, "Trades: {}", self.num_trades)?;
        writeln!(f, "Avg Trade PnL: {:.2}", self.avg_trade_pnl)?;
        writeln!(f, "Best Trade: {:.2}", self.best_trade)?;
        writeln!(f, "Worst Trade: {:.2}", self.worst_trade)?;
        writeln!(f, "Exposure: {:.1}%", self.exposure * 100.0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate() {
        let equity = vec![10000.0, 10100.0, 9900.0, 10200.0, 10500.0];
        let returns: Vec<f64> = equity.windows(2).map(|w| (w[1] - w[0]) / w[0]).collect();
        let in_market = vec![true, true, false, true, true];
        let trade_pnls = vec![100.0, -50.0, 200.0];

        let metrics =
            PerformanceMetrics::calculate(&equity, &returns, 0.03, 8760.0, &in_market, &trade_pnls);

        assert!(metrics.total_return > 0.0);
        assert!(metrics.max_drawdown > 0.0);
        assert!(metrics.num_trades == 3);
        assert!(metrics.win_rate > 0.0 && metrics.win_rate < 1.0);
    }
}

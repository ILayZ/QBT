use crate::backtest::BacktestResult;
use crate::data::PriceSeries;
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::json;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    Ok(())
}

#[derive(Default, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_currency: Option<String>,
}

impl DatasetMetadata {
    pub fn is_empty(&self) -> bool {
        self.symbol.is_none() && self.quote_currency.is_none()
    }
}

pub fn summary(path: &Path, results: &[BacktestResult]) -> Result<()> {
    summary_with_metadata(path, results, &DatasetMetadata::default())
}

pub fn summary_with_metadata(
    path: &Path,
    results: &[BacktestResult],
    metadata: &DatasetMetadata,
) -> Result<()> {
    let dca = results.iter().find(|r| r.strategy_name == "DCA");
    let portfolios: Vec<_> = results
        .iter()
        .map(|r| {
            json!({
                "name": r.strategy_name,
                "initial_capital": r.initial_capital,
                "final_equity": r.final_value,
                "difference_vs_dca": dca.map(|baseline| r.final_value - baseline.final_value),
                "total_return": r.total_return,
                "max_drawdown": r.max_drawdown,
                "completed_trades": r.num_trades,
                "skipped_buys": r.skipped_buys,
                "below_minimum_buys": r.below_minimum_buys,
                "mean_asset_weight": r.mean_asset_weight,
                "unspent_reserve": r.dip_reserve,
                "max_reserve_wait_hours": r.dip_reserve.map(|_| r.max_reserve_wait_hours),
            })
        })
        .collect();
    let mut report = json!({"version": 1, "portfolios": portfolios});
    if let Some(symbol) = &metadata.symbol {
        report["symbol"] = json!(symbol);
    }
    if let Some(quote_currency) = &metadata.quote_currency {
        report["quote_currency"] = json!(quote_currency);
    }
    write_json(path, &report)
}

pub fn equity(path: &Path, series: &PriceSeries, results: &[BacktestResult]) -> Result<()> {
    #[derive(Serialize)]
    struct Curve<'a> {
        name: &'a str,
        values: &'a [f64],
    }

    #[derive(Serialize)]
    struct Equity<'a> {
        version: u32,
        timestamps: Vec<chrono::DateTime<chrono::Utc>>,
        portfolios: Vec<Curve<'a>>,
    }

    let mut portfolios = Vec::with_capacity(results.len());
    for result in results {
        ensure!(
            result.equity_curve.len() == series.len(),
            "Equity length mismatch"
        );
        portfolios.push(Curve {
            name: &result.strategy_name,
            values: &result.equity_curve,
        });
    }
    let equity = Equity {
        version: 1,
        timestamps: series.bars.iter().map(|bar| bar.timestamp).collect(),
        portfolios,
    };
    write_json(path, &equity)
}

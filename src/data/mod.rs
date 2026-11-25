use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use polars::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// OHLCV bar data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    pub fn typical_price(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    pub fn vwap(&self) -> f64 {
        self.typical_price() * self.volume
    }
}

/// Price series for a symbol
#[derive(Debug, Clone)]
pub struct PriceSeries {
    pub symbol: String,
    pub bars: Vec<Bar>,
}

impl PriceSeries {
    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    pub fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.close).collect()
    }

    pub fn opens(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.open).collect()
    }

    pub fn returns(&self) -> Vec<f64> {
        self.bars
            .windows(2)
            .map(|w| (w[1].close - w[0].close) / w[0].close)
            .collect()
    }

    pub fn log_returns(&self) -> Vec<f64> {
        self.bars
            .windows(2)
            .map(|w| (w[1].close / w[0].close).ln())
            .collect()
    }

    pub fn to_dataframe(&self) -> Result<DataFrame> {
        let timestamps: Vec<i64> = self
            .bars
            .iter()
            .map(|b| b.timestamp.timestamp_millis())
            .collect();
        let opens: Vec<f64> = self.bars.iter().map(|b| b.open).collect();
        let highs: Vec<f64> = self.bars.iter().map(|b| b.high).collect();
        let lows: Vec<f64> = self.bars.iter().map(|b| b.low).collect();
        let closes: Vec<f64> = self.bars.iter().map(|b| b.close).collect();
        let volumes: Vec<f64> = self.bars.iter().map(|b| b.volume).collect();

        Ok(DataFrame::new(vec![
            Series::new("timestamp", timestamps)
                .cast(&DataType::Datetime(TimeUnit::Milliseconds, None))?,
            Series::new("open", opens),
            Series::new("high", highs),
            Series::new("low", lows),
            Series::new("close", closes),
            Series::new("volume", volumes),
        ])?)
    }

    pub fn from_dataframe(df: DataFrame, symbol: String) -> Result<Self> {
        let timestamp_col = df.column("timestamp")?.datetime()?;
        let time_unit = timestamp_col.time_unit();
        let open_col = df.column("open")?.f64()?;
        let high_col = df.column("high")?.f64()?;
        let low_col = df.column("low")?.f64()?;
        let close_col = df.column("close")?.f64()?;
        let volume_col = df.column("volume")?.f64()?;

        let mut bars = Vec::with_capacity(df.height());
        for i in 0..df.height() {
            let ts = timestamp_col.get(i).context("missing timestamp")?;
            // Normalize to milliseconds regardless of the column's time unit.
            let millis = match time_unit {
                TimeUnit::Milliseconds => ts,
                TimeUnit::Microseconds => ts / 1_000,
                TimeUnit::Nanoseconds => ts / 1_000_000,
            };
            let dt = DateTime::from_timestamp_millis(millis).context("invalid timestamp")?;
            bars.push(Bar {
                timestamp: dt,
                open: open_col.get(i).context("missing open")?,
                high: high_col.get(i).context("missing high")?,
                low: low_col.get(i).context("missing low")?,
                close: close_col.get(i).context("missing close")?,
                volume: volume_col.get(i).context("missing volume")?,
            });
        }

        Ok(Self { symbol, bars })
    }

    /// Load from CSV file
    pub fn from_csv(path: &Path, symbol: String) -> Result<Self> {
        let read_options = CsvReadOptions {
            has_header: true,
            parse_options: std::sync::Arc::new(
                polars::prelude::CsvParseOptions::default().with_try_parse_dates(true),
            ),
            ..Default::default()
        };
        let df = read_options
            .try_into_reader_with_file_path(Some(path.to_path_buf()))?
            .finish()?;
        Self::from_dataframe(df, symbol)
    }

    /// Load from Parquet file
    pub fn from_parquet(path: &Path, symbol: String) -> Result<Self> {
        let df = ParquetReader::new(std::fs::File::open(path)?).finish()?;
        Self::from_dataframe(df, symbol)
    }

    /// Save to Parquet
    pub fn to_parquet(&self, path: &Path) -> Result<()> {
        let mut df = self.to_dataframe()?;
        let mut file = std::fs::File::create(path)?;
        ParquetWriter::new(&mut file).finish(&mut df)?;
        Ok(())
    }
}

/// Multi-symbol data container
#[derive(Debug, Clone)]
pub struct MarketData {
    pub series: Vec<PriceSeries>,
}

impl MarketData {
    pub fn new() -> Self {
        Self { series: Vec::new() }
    }

    pub fn add(&mut self, series: PriceSeries) {
        self.series.push(series);
    }

    pub fn get(&self, symbol: &str) -> Option<&PriceSeries> {
        self.series.iter().find(|s| s.symbol == symbol)
    }

    pub fn symbols(&self) -> Vec<&str> {
        self.series.iter().map(|s| s.symbol.as_str()).collect()
    }

    pub fn align(&self) -> Result<AlignedData> {
        // Find common timestamp range
        let min_ts = self
            .series
            .iter()
            .filter_map(|s| s.bars.first().map(|b| b.timestamp))
            .max()
            .context("no data")?;
        let max_ts = self
            .series
            .iter()
            .filter_map(|s| s.bars.last().map(|b| b.timestamp))
            .min()
            .context("no data")?;

        // Filter each series to common range
        let mut aligned = Vec::new();
        for s in &self.series {
            let bars: Vec<Bar> = s
                .bars
                .iter()
                .filter(|b| b.timestamp >= min_ts && b.timestamp <= max_ts)
                .cloned()
                .collect();
            if !bars.is_empty() {
                aligned.push(PriceSeries {
                    symbol: s.symbol.clone(),
                    bars,
                });
            }
        }

        Ok(AlignedData {
            series: aligned,
            start: min_ts,
            end: max_ts,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AlignedData {
    pub series: Vec<PriceSeries>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Default for MarketData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_bar_calculations() {
        let bar = Bar {
            timestamp: Utc::now(),
            open: 100.0,
            high: 110.0,
            low: 95.0,
            close: 105.0,
            volume: 1000.0,
        };
        assert_eq!(bar.typical_price(), 103.33333333333333);
        assert_eq!(bar.vwap(), 103333.33333333333);
    }

    #[test]
    fn test_returns() {
        let bars = vec![
            Bar {
                timestamp: Utc.timestamp_opt(0, 0).unwrap(),
                open: 100.0,
                high: 100.0,
                low: 100.0,
                close: 100.0,
                volume: 100.0,
            },
            Bar {
                timestamp: Utc.timestamp_opt(3600, 0).unwrap(),
                open: 105.0,
                high: 105.0,
                low: 105.0,
                close: 105.0,
                volume: 100.0,
            },
            Bar {
                timestamp: Utc.timestamp_opt(7200, 0).unwrap(),
                open: 99.0,
                high: 99.0,
                low: 99.0,
                close: 99.0,
                volume: 100.0,
            },
        ];
        let series = PriceSeries {
            symbol: "TEST".into(),
            bars,
        };
        let rets = series.returns();
        assert_eq!(rets.len(), 2);
        assert!((rets[0] - 0.05).abs() < 1e-10);
        assert!((rets[1] - (-0.057142857)).abs() < 1e-6);
    }
}

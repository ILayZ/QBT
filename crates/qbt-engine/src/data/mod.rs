use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDateTime, Utc};
use polars::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

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
    /// Validate a single series of consecutive hourly OHLCV bars.
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.is_empty(), "Price series has no bars");
        for (i, pair) in self.bars.windows(2).enumerate() {
            let [previous, next] = [pair[0].timestamp, pair[1].timestamp];
            let position = i + 2;
            ensure!(
                next != previous,
                "Bar {position}: duplicate timestamp {next}"
            );
            ensure!(
                next > previous,
                "Bar {position}: timestamp {next} is out of order after {previous}"
            );
            ensure!(
                next - previous == chrono::Duration::hours(1),
                "Bar {position}: expected consecutive hourly bars, found {previous} followed by {next}"
            );
        }
        for (i, bar) in self.bars.iter().enumerate() {
            let position = i + 1;
            for (name, value) in [
                ("open", bar.open),
                ("high", bar.high),
                ("low", bar.low),
                ("close", bar.close),
            ] {
                ensure!(
                    value.is_finite() && value > 0.0,
                    "Bar {position}: {name} must be finite and positive (found {value})"
                );
            }
            ensure!(
                bar.low <= bar.open.min(bar.close) && bar.high >= bar.open.max(bar.close),
                "Bar {position}: low and high must bound open and close"
            );
            ensure!(
                bar.volume.is_finite() && bar.volume >= 0.0,
                "Bar {position}: volume must be finite and nonnegative (found {})",
                bar.volume
            );
        }
        Ok(())
    }

    /// Load and validate OHLCV data. A .parquet extension selects Parquet.
    pub fn from_path(path: &Path, symbol: String) -> Result<Self> {
        let series = if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("parquet"))
        {
            Self::from_parquet(path, symbol)
        } else {
            Self::from_csv(path, symbol)
        }?;
        series.validate()?;
        Ok(series)
    }

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
        let timestamp_col = df.column("timestamp").context("Missing timestamp column")?;
        let open_col = numeric_column(&df, "open")?;
        let high_col = numeric_column(&df, "high")?;
        let low_col = numeric_column(&df, "low")?;
        let close_col = numeric_column(&df, "close")?;
        let volume_col = numeric_column(&df, "volume")?;

        let mut bars = Vec::with_capacity(df.height());
        for i in 0..df.height() {
            let get = |column: &Float64Chunked| {
                column
                    .get(i)
                    .with_context(|| format!("Bar {}: missing {}", i + 1, column.name()))
            };
            bars.push(Bar {
                timestamp: read_timestamp(timestamp_col, i)
                    .with_context(|| format!("Bar {}: invalid timestamp", i + 1))?,
                open: get(&open_col)?,
                high: get(&high_col)?,
                low: get(&low_col)?,
                close: get(&close_col)?,
                volume: get(&volume_col)?,
            });
        }

        Ok(Self { symbol, bars })
    }

    /// Load from CSV file
    pub fn from_csv(path: &Path, symbol: String) -> Result<Self> {
        // Preserve timestamp precision and identify the offending bar on parse errors.
        let timestamp = Schema::from_iter([Field::new("timestamp", DataType::String)]);
        let df = CsvReadOptions::default()
            .with_schema_overwrite(Some(Arc::new(timestamp)))
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

fn numeric_column(df: &DataFrame, name: &str) -> Result<Float64Chunked> {
    let column = df
        .column(name)
        .with_context(|| format!("Missing {name} column"))?;
    if let Ok(strings) = column.str() {
        let parsed = strings
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                value
                    .map(|value| {
                        value
                            .trim()
                            .parse::<f64>()
                            .with_context(|| format!("Bar {}: {name} must be numeric", i + 1))
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(Float64Chunked::from_iter_options(name, parsed.into_iter()));
    }
    ensure!(
        column.dtype().is_numeric() || column.dtype().is_decimal(),
        "{name} must be numeric"
    );
    Ok(column.cast(&DataType::Float64)?.f64()?.clone())
}

fn read_timestamp(column: &Series, index: usize) -> Result<DateTime<Utc>> {
    match column.dtype() {
        DataType::Datetime(unit, _) => {
            let value = column.datetime()?.get(index).context("missing value")?;
            match unit {
                TimeUnit::Milliseconds => DateTime::from_timestamp_millis(value),
                TimeUnit::Microseconds => DateTime::from_timestamp_micros(value),
                TimeUnit::Nanoseconds => Some(DateTime::from_timestamp_nanos(value)),
            }
            .context("timestamp is out of range")
        }
        DataType::String => {
            let value = column.str()?.get(index).context("missing value")?.trim();
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
                return Ok(timestamp.with_timezone(&Utc));
            }
            // Older CSVs can omit the UTC offset or use a space separator.
            let value = value.replace(' ', "T");
            for format in [
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y-%m-%dT%H:%M",
                "%Y-%m-%dT%H%M%S%.f",
                "%Y-%m-%dT%H%M",
            ] {
                if let Ok(timestamp) = DateTime::parse_from_str(&value, &format!("{format}%#z")) {
                    return Ok(timestamp.with_timezone(&Utc));
                }
                if let Ok(timestamp) = NaiveDateTime::parse_from_str(&value, format) {
                    return Ok(timestamp.and_utc());
                }
            }
            bail!("expected an ISO 8601 datetime, found {value:?}")
        }
        _ => bail!("expected a datetime column or ISO 8601 datetime strings"),
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

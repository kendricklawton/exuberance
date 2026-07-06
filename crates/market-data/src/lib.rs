//! Market-data layer for `exuberance`.
//!
//! One tested place to get prices and implied-vol snapshots. Everything is
//! expressed against the [`DataSource`] trait so screeners never care whether
//! the bytes came from Polygon, a mock, or (later) a cache.
//!
//! Live market data also reaches the AI agents through the `massive` and Polygon
//! **MCP tools**; this crate is the *Rust* path used by the compiled engine.

/// A daily OHLCV bar. Timestamps are Unix epoch **seconds** (UTC, market close).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub t: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// An implied-volatility snapshot for a symbol, with enough history to rank it.
///
/// `iv` and each entry in `iv_history` are decimals (0.30 == 30% annualized),
/// typically the ATM / 30-day implied vol.
#[derive(Debug, Clone, PartialEq)]
pub struct IvSnapshot {
    pub symbol: String,
    pub iv: f64,
    /// Trailing IV observations (e.g. daily ATM IV over the last 1–3 years),
    /// used to compute IV rank / percentile.
    pub iv_history: Vec<f64>,
}

/// Errors a data source can return.
#[derive(Debug, Clone, PartialEq)]
pub enum DataError {
    /// Symbol not found by the provider.
    NotFound(String),
    /// Provider or transport failure (HTTP error, timeout, bad payload).
    Provider(String),
    /// Path not wired yet (e.g. live Polygon HTTP).
    NotImplemented(&'static str),
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::NotFound(s) => write!(f, "symbol not found: {s}"),
            DataError::Provider(s) => write!(f, "provider error: {s}"),
            DataError::NotImplemented(s) => write!(f, "not implemented yet: {s}"),
        }
    }
}

impl std::error::Error for DataError {}

/// Anything that can supply prices and IV. Screeners depend on this, not on a
/// concrete provider, so tests use [`MockSource`] and prod uses Polygon.
pub trait DataSource {
    /// Daily bars, oldest-first, covering roughly `lookback_days` sessions.
    fn daily_bars(&self, symbol: &str, lookback_days: usize) -> Result<Vec<Bar>, DataError>;

    /// Current IV plus trailing history for the symbol.
    fn iv_snapshot(&self, symbol: &str) -> Result<IvSnapshot, DataError>;
}

/// Convenience: pull the closing prices out of a bar series (oldest-first).
pub fn closes(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.close).collect()
}

/// In-memory data source for tests, demos, and backtest fixtures.
#[derive(Debug, Default, Clone)]
pub struct MockSource {
    entries: std::collections::HashMap<String, (Vec<Bar>, IvSnapshot)>,
}

impl MockSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a symbol from raw closing prices and an IV snapshot. Bars are
    /// synthesized with `close == open == high == low` and sequential timestamps.
    pub fn insert_from_closes(
        &mut self,
        symbol: &str,
        closes: &[f64],
        iv: f64,
        iv_history: Vec<f64>,
    ) {
        let bars = closes
            .iter()
            .enumerate()
            .map(|(i, &c)| Bar {
                t: i as i64,
                open: c,
                high: c,
                low: c,
                close: c,
                volume: 0.0,
            })
            .collect();
        let snap = IvSnapshot {
            symbol: symbol.to_string(),
            iv,
            iv_history,
        };
        self.entries.insert(symbol.to_string(), (bars, snap));
    }
}

impl DataSource for MockSource {
    fn daily_bars(&self, symbol: &str, lookback_days: usize) -> Result<Vec<Bar>, DataError> {
        let (bars, _) = self
            .entries
            .get(symbol)
            .ok_or_else(|| DataError::NotFound(symbol.to_string()))?;
        let start = bars.len().saturating_sub(lookback_days);
        Ok(bars[start..].to_vec())
    }

    fn iv_snapshot(&self, symbol: &str) -> Result<IvSnapshot, DataError> {
        self.entries
            .get(symbol)
            .map(|(_, snap)| snap.clone())
            .ok_or_else(|| DataError::NotFound(symbol.to_string()))
    }
}

/// Live Polygon.io data source. Skeleton only — HTTP wiring is the next
/// milestone (see CLAUDE.md "Build order"). Reads `POLYGON_API_KEY` from env.
#[derive(Debug, Clone)]
pub struct PolygonSource {
    #[allow(dead_code)]
    api_key: String,
}

impl PolygonSource {
    /// Construct from the `POLYGON_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, DataError> {
        let api_key = std::env::var("POLYGON_API_KEY")
            .map_err(|_| DataError::Provider("POLYGON_API_KEY not set".into()))?;
        Ok(Self { api_key })
    }
}

impl DataSource for PolygonSource {
    fn daily_bars(&self, _symbol: &str, _lookback_days: usize) -> Result<Vec<Bar>, DataError> {
        // TODO: GET /v2/aggs/ticker/{symbol}/range/1/day/{from}/{to}
        Err(DataError::NotImplemented("PolygonSource::daily_bars"))
    }

    fn iv_snapshot(&self, _symbol: &str) -> Result<IvSnapshot, DataError> {
        // TODO: pull ATM IV from the options snapshot endpoint + build history.
        Err(DataError::NotImplemented("PolygonSource::iv_snapshot"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_roundtrips_closes_and_iv() {
        let mut src = MockSource::new();
        src.insert_from_closes("AAA", &[10.0, 11.0, 12.0], 0.25, vec![0.2, 0.3, 0.4]);

        let bars = src.daily_bars("AAA", 100).unwrap();
        assert_eq!(closes(&bars), vec![10.0, 11.0, 12.0]);

        let snap = src.iv_snapshot("AAA").unwrap();
        assert_eq!(snap.iv, 0.25);
        assert_eq!(snap.iv_history.len(), 3);

        assert_eq!(src.daily_bars("ZZZ", 10), Err(DataError::NotFound("ZZZ".into())));
    }

    #[test]
    fn lookback_truncates_to_most_recent() {
        let mut src = MockSource::new();
        src.insert_from_closes("BBB", &[1.0, 2.0, 3.0, 4.0, 5.0], 0.2, vec![]);
        let bars = src.daily_bars("BBB", 2).unwrap();
        assert_eq!(closes(&bars), vec![4.0, 5.0]);
    }
}

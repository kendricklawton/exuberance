//! The IV-history store seam + the acquisition decorator (ROADMAP Phase 8 — the reason to
//! exist).
//!
//! IV *rank* needs a trailing IV *series* that a snapshot call can't return; the engine
//! persists observations and ranks against the accumulated distribution. [`IvStore`] is the
//! persistence **port** (a SQLite adapter lives in `crates/store`; [`MemoryIvStore`] here is
//! the dep-free in-memory one for tests and lean builds). [`StoreBackedSource`] is the
//! composition: it wraps any feed + a store and, keyed on
//! [`iv_history_strategy`], either **accumulates** a snapshot
//! feed forward or **backfills** from a history-capable feed — the screen sees an identical
//! [`IvSnapshot`] either way (the anti-corruption layer).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::{ProviderError, ProviderResult};
use crate::market_data::{
    iv_history_strategy, Bar, IvHistoryMeta, IvHistoryStrategy, IvSnapshot, MarketDataProvider,
};
use crate::provider::{Provider, ProviderInfo};

/// Persistence for daily ATM IV observations, keyed by `(symbol, date_days)`. `Send + Sync`
/// so a runtime-selected `Arc<dyn IvStore>` can be shared across the engine's async tasks.
#[async_trait]
pub trait IvStore: Send + Sync {
    /// Record one day's ATM IV. Idempotent per `(symbol, date_days)`: re-recording the same
    /// day overwrites, so two scans on one day yield one observation.
    async fn record_iv(&self, symbol: &str, date_days: i64, iv: f64) -> ProviderResult<()>;

    /// Record many observations at once (a backfill batch).
    async fn record_iv_batch(&self, symbol: &str, obs: &[(i64, f64)]) -> ProviderResult<()>;

    /// The stored IV series for a symbol, oldest→newest as `(date_days, iv)`.
    async fn iv_series(&self, symbol: &str) -> ProviderResult<Vec<(i64, f64)>>;
}

/// In-memory [`IvStore`] — dep-free, for tests and the lean (no-`sqlite`) build. Not
/// persistent across process runs; the SQLite adapter is for that.
#[derive(Debug, Default)]
pub struct MemoryIvStore {
    // symbol → (date_days → iv); the BTreeMap keeps observations sorted + deduped by date.
    inner: Mutex<HashMap<String, BTreeMap<i64, f64>>>,
}

impl MemoryIvStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(
        &self,
    ) -> ProviderResult<std::sync::MutexGuard<'_, HashMap<String, BTreeMap<i64, f64>>>> {
        self.inner
            .lock()
            .map_err(|_| ProviderError::Transport("in-memory IV store lock poisoned".into()))
    }
}

#[async_trait]
impl IvStore for MemoryIvStore {
    async fn record_iv(&self, symbol: &str, date_days: i64, iv: f64) -> ProviderResult<()> {
        self.lock()?
            .entry(symbol.to_string())
            .or_default()
            .insert(date_days, iv);
        Ok(())
    }

    async fn record_iv_batch(&self, symbol: &str, obs: &[(i64, f64)]) -> ProviderResult<()> {
        let mut guard = self.lock()?;
        let series = guard.entry(symbol.to_string()).or_default();
        for &(date, iv) in obs {
            series.insert(date, iv);
        }
        Ok(())
    }

    async fn iv_series(&self, symbol: &str) -> ProviderResult<Vec<(i64, f64)>> {
        Ok(self
            .lock()?
            .get(symbol)
            .map(|m| m.iter().map(|(&d, &v)| (d, v)).collect())
            .unwrap_or_default())
    }
}

/// A feed decorated with a store: it turns any [`MarketDataProvider`] into one whose
/// `iv_snapshot` carries a *ranked distribution* assembled by the store, capability-driven.
pub struct StoreBackedSource {
    inner: Box<dyn MarketDataProvider>,
    store: Arc<dyn IvStore>,
    /// Today, as days since the Unix epoch — the date accumulation records under. A field so
    /// tests can pin it (real construction stamps it from the clock).
    today_days: i64,
    /// How far back to ask a backfill feed for history.
    backfill_days: usize,
}

impl StoreBackedSource {
    /// Wrap `inner` with `store`, stamping today from the system clock.
    #[must_use]
    pub fn new(inner: Box<dyn MarketDataProvider>, store: Arc<dyn IvStore>) -> Self {
        Self {
            inner,
            store,
            today_days: today_days(),
            backfill_days: 756, // ~3 years of trading days
        }
    }

    /// Pin "today" (epoch days) — tests use this to prove the window grows across days.
    #[must_use]
    pub fn with_today_days(mut self, today_days: i64) -> Self {
        self.today_days = today_days;
        self
    }

    /// Assemble the snapshot the screen sees: current `iv`, the store's series as the ranking
    /// history, and provenance citing how it was built.
    async fn assemble(&self, symbol: &str, iv: f64, source: &str) -> ProviderResult<IvSnapshot> {
        let series = self.store.iv_series(symbol).await?;
        let values: Vec<f64> = series.iter().map(|&(_, v)| v).collect();
        let span = match (series.first(), series.last()) {
            (Some(&(first, _)), Some(&(last, _))) => last - first,
            _ => 0,
        };
        let meta = IvHistoryMeta::new(series.len(), span, source);
        Ok(IvSnapshot::new(symbol, iv, values).with_history(meta))
    }
}

impl Provider for StoreBackedSource {
    fn info(&self) -> ProviderInfo {
        // Identity is the underlying feed's — the store is transparent composition.
        self.inner.info()
    }
}

#[async_trait]
impl MarketDataProvider for StoreBackedSource {
    async fn daily_bars(&self, symbol: &str, lookback_days: usize) -> ProviderResult<Vec<Bar>> {
        // Bar caching is Phase 9 (the caching/fan-out theme); here we just delegate.
        self.inner.daily_bars(symbol, lookback_days).await
    }

    async fn iv_snapshot(&self, symbol: &str) -> ProviderResult<IvSnapshot> {
        let id = self.inner.info().id;
        match iv_history_strategy(self.inner.as_ref()) {
            IvHistoryStrategy::Accumulate => {
                // Snapshot-only feed: record today's IV, rank against the growing series.
                let raw = self.inner.iv_snapshot(symbol).await?;
                self.store
                    .record_iv(symbol, self.today_days, raw.iv)
                    .await?;
                self.assemble(symbol, raw.iv, &format!("accumulated ({id})"))
                    .await
            }
            IvHistoryStrategy::Backfill => {
                // History-capable feed: fill the distribution once, then keep it current.
                if self.store.iv_series(symbol).await?.is_empty() {
                    let history = self.inner.iv_history(symbol, self.backfill_days).await?;
                    let batch: Vec<(i64, f64)> =
                        history.iter().map(|o| (o.date_days, o.iv)).collect();
                    self.store.record_iv_batch(symbol, &batch).await?;
                }
                let raw = self.inner.iv_snapshot(symbol).await?;
                self.store
                    .record_iv(symbol, self.today_days, raw.iv)
                    .await?;
                self.assemble(symbol, raw.iv, &format!("backfilled ({id})"))
                    .await
            }
        }
    }
}

/// Today as days since the Unix epoch (UTC). `0` before the epoch — unreachable in practice.
#[must_use]
pub fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::IvObservation;
    use crate::provider::{Capability, ProviderKind};

    /// A snapshot-only feed (no OptionsHistory → Accumulate): returns a fixed current IV and
    /// empty history, like a real snapshot vendor.
    struct SnapshotFeed {
        iv: f64,
    }
    impl Provider for SnapshotFeed {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                id: "snap".into(),
                kind: ProviderKind::MarketData,
                capabilities: vec![Capability::ImpliedVol],
            }
        }
    }
    #[async_trait]
    impl MarketDataProvider for SnapshotFeed {
        async fn daily_bars(&self, _s: &str, _n: usize) -> ProviderResult<Vec<Bar>> {
            Ok(vec![])
        }
        async fn iv_snapshot(&self, symbol: &str) -> ProviderResult<IvSnapshot> {
            Ok(IvSnapshot::new(symbol, self.iv, vec![]))
        }
    }

    /// A history-capable feed (advertises OptionsHistory → Backfill): serves a synthetic
    /// multi-year daily IV series in one call.
    struct BackfillFeed {
        series: Vec<IvObservation>,
    }
    impl Provider for BackfillFeed {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                id: "backfill".into(),
                kind: ProviderKind::MarketData,
                capabilities: vec![Capability::ImpliedVol, Capability::OptionsHistory],
            }
        }
    }
    #[async_trait]
    impl MarketDataProvider for BackfillFeed {
        async fn daily_bars(&self, _s: &str, _n: usize) -> ProviderResult<Vec<Bar>> {
            Ok(vec![])
        }
        async fn iv_snapshot(&self, symbol: &str) -> ProviderResult<IvSnapshot> {
            let iv = self.series.last().map_or(0.3, |o| o.iv);
            Ok(IvSnapshot::new(symbol, iv, vec![]))
        }
        async fn iv_history(&self, _s: &str, _n: usize) -> ProviderResult<Vec<IvObservation>> {
            Ok(self.series.clone())
        }
    }

    #[tokio::test]
    async fn accumulate_grows_the_window_across_days() {
        let store = Arc::new(MemoryIvStore::new());

        // Day 1: one observation, no range to rank yet.
        let d1 = StoreBackedSource::new(Box::new(SnapshotFeed { iv: 0.20 }), store.clone())
            .with_today_days(100);
        let s1 = d1.iv_snapshot("AAA").await.unwrap();
        let m1 = s1.history.unwrap();
        assert_eq!(m1.observations, 1);
        assert_eq!(m1.span_days, 0);
        assert!(m1.source.contains("accumulated") && m1.source.contains("snap"));

        // Day 2 (same store): the window is now two observations spanning one day.
        let d2 = StoreBackedSource::new(Box::new(SnapshotFeed { iv: 0.35 }), store.clone())
            .with_today_days(101);
        let s2 = d2.iv_snapshot("AAA").await.unwrap();
        let m2 = s2.history.unwrap();
        assert_eq!(m2.observations, 2);
        assert_eq!(m2.span_days, 1);
        assert_eq!(s2.iv_history, vec![0.20, 0.35]);
    }

    #[tokio::test]
    async fn same_day_rescan_is_idempotent() {
        let store = Arc::new(MemoryIvStore::new());
        let make = |iv| {
            StoreBackedSource::new(Box::new(SnapshotFeed { iv }), store.clone())
                .with_today_days(100)
        };
        make(0.20).iv_snapshot("AAA").await.unwrap();
        let again = make(0.25).iv_snapshot("AAA").await.unwrap();
        // One day → one observation, overwritten to the latest value.
        assert_eq!(again.history.unwrap().observations, 1);
        assert_eq!(again.iv_history, vec![0.25]);
    }

    #[tokio::test]
    async fn backfill_fills_a_multi_year_distribution_in_one_call() {
        // ~260 daily observations spanning ~370 days.
        let series: Vec<IvObservation> = (0..260)
            .map(|i| IvObservation::new(1000 + i, 0.20 + (i as f64) * 0.001))
            .collect();
        let store = Arc::new(MemoryIvStore::new());
        let src =
            StoreBackedSource::new(Box::new(BackfillFeed { series }), store).with_today_days(2000);

        let snap = src.iv_snapshot("AAA").await.unwrap();
        let meta = snap.history.unwrap();
        // The 260 backfilled obs plus today's snapshot record.
        assert!(meta.observations >= 260, "got {}", meta.observations);
        assert!(meta.span_days >= 365, "got {}", meta.span_days);
        assert!(meta.source.contains("backfilled"));
    }
}

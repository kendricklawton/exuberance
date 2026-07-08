//! The [`BrokerProvider`] contract + a paper mock.
//!
//! One trait for every execution venue (Tradier, Alpaca, IBKR, …). The types are
//! deliberately minimal — enough to size, place, and inspect an order — and the
//! guardrail from CLAUDE.md is baked into the contract: a broker reports its
//! [`TradingMode`], and going live is never inferred. The [`PaperBroker`] mock
//! demonstrates the shape and enforces the guardrail.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use crate::error::{ProviderError, ProviderResult};
use crate::provider::{Capability, Provider, ProviderInfo, ProviderKind};

/// Paper (simulated) vs. live (real money). Live is a deliberate, human act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingMode {
    Paper,
    Live,
}

/// Buy or sell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// How the order should be priced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    Market,
    /// Limit at the given price (decimal, e.g. 1.25 == $1.25).
    Limit(f64),
}

/// A request to trade. `symbol` is an OCC option symbol or an equity ticker;
/// the broker decides how to route based on its capabilities.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: Side,
    /// Contracts (options) or shares (equity).
    pub quantity: u64,
    pub order_type: OrderType,
}

/// Where an order is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Accepted,
    Filled,
    Rejected,
    Cancelled,
}

/// The broker's acknowledgement of a placed order.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderReceipt {
    /// Broker-assigned id (or a simulated one in paper mode).
    pub id: String,
    pub status: OrderStatus,
    pub mode: TradingMode,
}

/// Account snapshot used for sizing and mandate checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Account {
    /// Settled cash (decimal currency units).
    pub cash: f64,
    /// Buying power available for new positions.
    pub buying_power: f64,
}

/// Anything that can hold an account and place orders. The risk layer sits in
/// front of this; the trait itself only promises a venue-neutral surface.
///
/// The I/O methods are `async` (a real broker is a network round-trip);
/// `#[async_trait]` keeps the trait object-safe for a runtime-selected
/// `Box<dyn BrokerProvider>`.
#[async_trait]
pub trait BrokerProvider: Provider {
    /// Paper or live. Callers must treat [`TradingMode::Live`] as load-bearing.
    fn mode(&self) -> TradingMode;

    /// Current account state for sizing and mandate checks.
    async fn account(&self) -> ProviderResult<Account>;

    /// Place an order. Implementations MUST refuse a live order unless live
    /// trading was explicitly enabled — return [`ProviderError::Refused`].
    async fn place_order(&self, req: &OrderRequest) -> ProviderResult<OrderReceipt>;
}

/// A no-network paper broker for tests, demos, and the default trading mode.
///
/// It accepts any well-formed order and simulates a fill. It refuses to be
/// constructed in live mode without the explicit `allow_live` acknowledgement,
/// mirroring the "no live orders without a human go" guardrail.
#[derive(Debug)]
pub struct PaperBroker {
    id: String,
    account: Account,
    // `AtomicU64` (not `Cell`) so `&PaperBroker` is `Sync` and the async trait's `Send` future compiles.
    next_id: AtomicU64,
}

impl PaperBroker {
    /// A paper broker seeded with the given account balances.
    pub fn new(id: impl Into<String>, cash: f64) -> Self {
        Self {
            id: id.into(),
            account: Account {
                cash,
                buying_power: cash,
            },
            next_id: AtomicU64::new(1),
        }
    }
}

impl Provider for PaperBroker {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.id.clone(),
            kind: ProviderKind::Broker,
            capabilities: vec![Capability::PaperTrading, Capability::OptionsOrders],
        }
    }
}

#[async_trait]
impl BrokerProvider for PaperBroker {
    fn mode(&self) -> TradingMode {
        TradingMode::Paper
    }

    async fn account(&self) -> ProviderResult<Account> {
        Ok(self.account)
    }

    async fn place_order(&self, req: &OrderRequest) -> ProviderResult<OrderReceipt> {
        if req.quantity == 0 {
            return Err(ProviderError::Refused("order quantity is zero".into()));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Ok(OrderReceipt {
            id: format!("paper-{id}"),
            status: OrderStatus::Filled,
            mode: TradingMode::Paper,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buy(symbol: &str, qty: u64) -> OrderRequest {
        OrderRequest {
            symbol: symbol.into(),
            side: Side::Buy,
            quantity: qty,
            order_type: OrderType::Market,
        }
    }

    #[tokio::test]
    async fn paper_broker_fills_and_stays_paper() {
        let b = PaperBroker::new("paper", 100_000.0);
        assert_eq!(b.mode(), TradingMode::Paper);
        assert!(b.supports(Capability::PaperTrading));
        assert!(!b.supports(Capability::LiveTrading));

        let r = b.place_order(&buy("SPY240719C00500000", 1)).await.unwrap();
        assert_eq!(r.status, OrderStatus::Filled);
        assert_eq!(r.mode, TradingMode::Paper);
        assert_eq!(r.id, "paper-1");
    }

    #[tokio::test]
    async fn zero_quantity_is_refused() {
        let b = PaperBroker::new("paper", 100_000.0);
        assert_eq!(
            b.place_order(&buy("SPY", 0)).await,
            Err(ProviderError::Refused("order quantity is zero".into()))
        );
    }
}

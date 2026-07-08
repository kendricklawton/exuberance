//! `exub-core` — the provider-agnostic contract layer for exuberance.
//!
//! The product is meant to be **modular and vendor-neutral**: it should run on
//! Massive or another feed, execute through Tradier or Alpaca or IBKR, and reason
//! with Claude or another model — without any of the trading logic knowing which.
//! This crate is where that neutrality is defined. It holds nothing but traits
//! and plain data types:
//!
//! - [`Provider`] + [`Capability`] — identity and feature-detection every vendor shares.
//! - [`MarketDataProvider`] — prices and implied vol ([`Bar`], [`IvSnapshot`]).
//! - [`BrokerProvider`] — accounts and order placement ([`OrderRequest`], [`TradingMode`]).
//! - [`AiProvider`] — completions for the reasoning layer ([`Prompt`], [`Completion`]).
//! - [`ProviderError`] — the single error vocabulary all of them speak.
//!
//! Concrete providers live in their own crates (`market-data`, and later `broker`
//! and `ai`), depend on this one, and are the only place a vendor is named. Higher
//! layers (`signals`, the CLI, the orchestrator) depend on the traits alone.
//!
//! `core` stays dependency-free and offline-testable — see CLAUDE.md conventions.

pub mod ai;
pub mod broker;
pub mod error;
pub mod market_data;
pub mod provider;

pub use ai::{AiProvider, Completion, EchoAi, Message, Prompt, Role};
pub use broker::{
    Account, BrokerProvider, OrderReceipt, OrderRequest, OrderStatus, OrderType, PaperBroker, Side,
    TradingMode,
};
pub use error::{ProviderError, ProviderResult};
pub use market_data::{
    closes, iv_history_strategy, Bar, IvHistoryStrategy, IvSnapshot, MarketDataProvider,
};
pub use provider::{Capability, Provider, ProviderInfo, ProviderKind};

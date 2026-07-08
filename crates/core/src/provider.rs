//! The base [`Provider`] contract and capability model.
//!
//! Every plugged-in vendor — Massive, Tradier, Alpaca, Anthropic, a local mock —
//! implements [`Provider`] so the engine can identify it and ask *what it can do*
//! before it asks it to do anything. Capability probing is how we stay agnostic:
//! screeners and the orchestrator branch on [`Capability`], never on a vendor name.

/// Which family a provider belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// Prices, bars, quotes, IV snapshots, chains (Massive, Alpha Vantage, …).
    MarketData,
    /// Accounts, positions, order placement/execution (Tradier, Alpaca, …).
    Broker,
    /// A raw LLM completion/reasoning API (Claude, Gemini, OpenAI, …).
    Ai,
    /// A coding/agentic assistant driven as a provider (Claude Code, Gemini CLI, OpenAI Codex, …). It
    /// speaks the same [`AiProvider`](crate::AiProvider) seam as a raw model, but advertises
    /// [`Capability::CodingAgent`] so callers know it runs an agentic loop rather than a single completion.
    Agent,
}

/// A discrete thing a provider may or may not be able to do.
///
/// This is the vocabulary of feature-detection. A provider advertises its
/// capabilities in [`ProviderInfo`]; callers check before relying on one, so a
/// data feed without options data degrades gracefully instead of erroring deep
/// in a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    // Market data
    DailyBars,
    IntradayBars,
    Quotes,
    OptionsChain,
    /// Serves a *current* implied-vol snapshot (rank must be built by accumulating these forward).
    ImpliedVol,
    /// Serves *historical* option chains with IV (e.g. Alpha Vantage) — lets the engine **backfill** the
    /// trailing IV distribution instead of accumulating it forward. See [`crate::iv_history_strategy`].
    OptionsHistory,
    // Broker
    PaperTrading,
    LiveTrading,
    OptionsOrders,
    StreamingFills,
    // AI models
    TextCompletion,
    ToolUse,
    Streaming,
    // AI coding agents (Claude Code, Gemini CLI, Codex, …)
    CodingAgent,
}

/// Identity + capability card for a provider instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Stable machine id, e.g. `"massive"`, `"tradier-paper"`, `"anthropic"`.
    pub id: String,
    /// Which family this provider serves.
    pub kind: ProviderKind,
    /// Everything this instance can do. Callers probe with [`Provider::supports`].
    pub capabilities: Vec<Capability>,
}

/// The root trait every provider implements. Object-safe on purpose so a
/// registry can hold `Box<dyn Provider>` of mixed vendors.
pub trait Provider {
    /// Identity and capability card for this instance.
    fn info(&self) -> ProviderInfo;

    /// Whether this provider advertises `cap`. Default checks [`ProviderInfo`].
    fn supports(&self, cap: Capability) -> bool {
        self.info().capabilities.contains(&cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;
    impl Provider for Dummy {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                id: "dummy".into(),
                kind: ProviderKind::MarketData,
                capabilities: vec![Capability::DailyBars, Capability::ImpliedVol],
            }
        }
    }

    #[test]
    fn supports_reads_capability_card() {
        let d = Dummy;
        assert!(d.supports(Capability::DailyBars));
        assert!(d.supports(Capability::ImpliedVol));
        assert!(!d.supports(Capability::OptionsChain));
    }
}

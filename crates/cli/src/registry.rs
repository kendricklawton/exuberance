//! The adapter registry — the **one place** a data provider, AI model, or coding agent is named.
//!
//! This is what makes exuberance a pluggable, multi-vendor engine: the config resolves a *name*
//! (`"mock"`, `"massive"`, `"claude"`, `"claude-code"`, …) and the registry maps it to a boxed trait object
//! the engine drives. Adding a vendor is a new adapter crate + one arm here — nothing in the core or the
//! screens changes. The [`catalog`] lists every intended vendor and whether it's wired yet, so
//! `exub providers` shows the plug-in matrix at a glance.

use anyhow::{bail, Result};
use exub_core::{AiProvider, EchoAi, MarketDataProvider, ProviderKind};
use market_data::MockSource;

/// Whether a catalog entry is actually implemented, or planned on the roadmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Implemented and selectable today.
    Wired,
    /// A named plug-in point that isn't implemented yet (see ROADMAP.md).
    Planned,
}

impl Status {
    /// A short tag for display.
    pub fn tag(self) -> &'static str {
        match self {
            Status::Wired => "wired",
            Status::Planned => "planned",
        }
    }
}

/// One vendor the engine can (or will) plug in.
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    /// The name used in config / `--data-provider` / `--ai-provider`.
    pub name: &'static str,
    /// Which seam it plugs into.
    pub kind: ProviderKind,
    /// Wired today, or planned.
    pub status: Status,
    /// A human note (what it is).
    pub note: &'static str,
}

/// The full plug-in catalog across every seam — data feeds, AI models, AI coding agents, and brokers.
/// This is the menu `exub providers` renders; `build_*` below turns a wired name into a live adapter.
pub fn catalog() -> Vec<CatalogEntry> {
    use ProviderKind::{Agent, Ai, Broker, MarketData};
    use Status::{Planned, Wired};
    vec![
        // Market-data feeds.
        CatalogEntry {
            name: "mock",
            kind: MarketData,
            status: Wired,
            note: "in-memory demo/test fixture",
        },
        CatalogEntry {
            name: "massive",
            kind: MarketData,
            status: Planned,
            note: "Massive (formerly Polygon.io) — licensed feed; IV snapshot → accumulate history",
        },
        CatalogEntry {
            name: "alpha-vantage",
            kind: MarketData,
            status: Planned,
            note:
                "Alpha Vantage — historical options w/ IV → backfill history (heavily rate-limited)",
        },
        // AI models (raw completion APIs).
        CatalogEntry {
            name: "mock",
            kind: Ai,
            status: Wired,
            note: "deterministic echo model (keyless)",
        },
        CatalogEntry {
            name: "claude",
            kind: Ai,
            status: Planned,
            note: "Anthropic Claude (Messages API)",
        },
        CatalogEntry {
            name: "gemini",
            kind: Ai,
            status: Planned,
            note: "Google Gemini (generateContent)",
        },
        CatalogEntry {
            name: "openai",
            kind: Ai,
            status: Planned,
            note: "OpenAI (Chat Completions)",
        },
        // AI coding agents (agentic CLIs) — same AiProvider seam, CodingAgent capability.
        CatalogEntry {
            name: "claude-code",
            kind: Agent,
            status: Planned,
            note: "Anthropic Claude Code (agentic CLI)",
        },
        CatalogEntry {
            name: "gemini-cli",
            kind: Agent,
            status: Planned,
            note: "Google Gemini CLI",
        },
        CatalogEntry {
            name: "codex",
            kind: Agent,
            status: Planned,
            note: "OpenAI Codex CLI",
        },
        // Brokers (human-initiated, paper-first execution — never the engine's job).
        CatalogEntry {
            name: "paper",
            kind: Broker,
            status: Wired,
            note: "no-network paper broker (default)",
        },
        CatalogEntry {
            name: "tradier",
            kind: Broker,
            status: Planned,
            note: "Tradier (paper + live)",
        },
        CatalogEntry {
            name: "alpaca",
            kind: Broker,
            status: Planned,
            note: "Alpaca (paper + live)",
        },
    ]
}

/// The wired names for a kind, for an actionable "unknown/planned provider" message.
fn wired_names(kind: ProviderKind) -> Vec<&'static str> {
    catalog()
        .into_iter()
        .filter(|e| e.kind == kind && e.status == Status::Wired)
        .map(|e| e.name)
        .collect()
}

/// Resolve a market-data adapter by name. `mock` returns a demo-seeded source; everything else is a named
/// but not-yet-wired plug-in point (a clear error, not a silent fallback).
///
/// # Errors
/// If `name` is unknown or catalogued-but-planned.
pub fn build_data_provider(name: &str) -> Result<Box<dyn MarketDataProvider>> {
    match name {
        "mock" => Ok(Box::new(MockSource::demo())),
        other => bail!(
            "data provider '{other}' is not wired yet (available: {}). \
             It's a planned plug-in point — see ROADMAP.md.",
            wired_names(ProviderKind::MarketData).join(", ")
        ),
    }
}

/// Resolve an AI adapter (model or coding agent) by name. `mock` returns the keyless echo provider.
///
/// The counterpart to [`build_data_provider`], ready for the AI-driven command (e.g. `ask`) that lands with
/// the real model adapters — exercised by the registry tests today, hence `allow(dead_code)` until then.
///
/// # Errors
/// If `name` is unknown or catalogued-but-planned.
#[allow(dead_code)]
pub fn build_ai_provider(name: &str) -> Result<Box<dyn AiProvider>> {
    match name {
        "mock" => Ok(Box::new(EchoAi::new("echo", "echo-1"))),
        other => bail!(
            "ai provider '{other}' is not wired yet (available: {}). \
             It's a planned plug-in point — see ROADMAP.md.",
            wired_names(ProviderKind::Ai).join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_seam_with_the_named_vendors() {
        let c = catalog();
        let has =
            |name: &str, kind: ProviderKind| c.iter().any(|e| e.name == name && e.kind == kind);
        // The vendors the product targets are all present as plug-in points.
        assert!(has("massive", ProviderKind::MarketData));
        assert!(has("alpha-vantage", ProviderKind::MarketData));
        assert!(has("claude", ProviderKind::Ai));
        assert!(has("gemini", ProviderKind::Ai));
        assert!(has("openai", ProviderKind::Ai));
        assert!(has("claude-code", ProviderKind::Agent));
        assert!(has("gemini-cli", ProviderKind::Agent));
        assert!(has("codex", ProviderKind::Agent));
    }

    #[tokio::test]
    async fn build_resolves_mock_and_rejects_planned() {
        // The wired defaults build and actually work.
        let data = build_data_provider("mock").unwrap();
        assert!(data.daily_bars("MOVER", 100).await.is_ok());
        assert!(build_ai_provider("mock").is_ok());

        // A catalogued-but-planned vendor is a clear error, not a silent fallback.
        // `.err()` (not `unwrap_err`) because the Ok type `Box<dyn MarketDataProvider>` isn't `Debug`.
        let err = build_data_provider("massive")
            .err()
            .expect("a planned provider must error")
            .to_string();
        assert!(
            err.contains("not wired yet") && err.contains("mock"),
            "{err}"
        );
        assert!(build_ai_provider("claude").is_err());
    }
}

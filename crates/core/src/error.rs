//! The one error type every provider speaks.
//!
//! Data feeds, brokers, and AI providers all fail in the same handful of ways —
//! a symbol/resource is missing, credentials are bad, the transport blew up, the
//! capability isn't offered, or it simply isn't wired yet. Collapsing those into
//! a single enum means callers (screeners, the CLI, the eventual orchestrator)
//! handle failure uniformly regardless of which vendor is plugged in.

use std::time::Duration;

/// A failure from any provider — market data, broker, or AI.
///
/// `#[non_exhaustive]` so new failure modes can be added without breaking downstream
/// `match`es (they must carry a `_` arm) — the anti-corruption boundary evolves additively.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    /// The requested symbol/order/resource does not exist at the provider.
    NotFound(String),
    /// Authentication or authorization failed (missing/invalid API key, scope).
    Auth(String),
    /// The provider is rate-limiting us. `retry_after` carries the backoff hint when the provider
    /// advertised one (e.g. a `Retry-After` header) — the seam retry/backoff logic keys off it.
    RateLimited {
        /// How long to wait before retrying, if the provider said.
        retry_after: Option<Duration>,
    },
    /// Transport or provider-side failure (HTTP error, timeout, bad payload).
    Transport(String),
    /// The provider does not offer this capability at all (see [`crate::Capability`]).
    Unsupported(&'static str),
    /// A guardrail refused the call (e.g. a live order without explicit go).
    Refused(String),
    /// Path exists in the contract but isn't implemented on this provider yet.
    NotImplemented(&'static str),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::NotFound(s) => write!(f, "not found: {s}"),
            ProviderError::Auth(s) => write!(f, "auth error: {s}"),
            ProviderError::RateLimited {
                retry_after: Some(d),
            } => {
                write!(f, "rate limited (retry after {}s)", d.as_secs())
            }
            ProviderError::RateLimited { retry_after: None } => write!(f, "rate limited"),
            ProviderError::Transport(s) => write!(f, "transport error: {s}"),
            ProviderError::Unsupported(s) => write!(f, "capability not supported: {s}"),
            ProviderError::Refused(s) => write!(f, "refused by guardrail: {s}"),
            ProviderError::NotImplemented(s) => write!(f, "not implemented yet: {s}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Shorthand for provider results.
pub type ProviderResult<T> = Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_display_includes_backoff() {
        assert_eq!(
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }
            .to_string(),
            "rate limited (retry after 7s)"
        );
        assert_eq!(
            ProviderError::RateLimited { retry_after: None }.to_string(),
            "rate limited"
        );
    }
}

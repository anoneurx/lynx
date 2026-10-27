//! Application state.

use std::sync::Arc;

use lynx_common::time::Clock;
use lynx_indexer::SearchIndexReader;
use lynx_ranker::Ranker;
use time::OffsetDateTime;

use crate::answer::AnswerClient;
use crate::middleware::SecurityContext;

#[derive(Clone)]
pub struct AppState {
    pub reader: Arc<dyn SearchIndexReader>,
    pub ranker: Ranker,
    pub clock: Option<Arc<dyn Clock>>,
    /// When the process started serving, for `/status` uptime.
    pub started_at: OffsetDateTime,
    /// Rate-limiting and header settings shared by every request.
    pub security: Arc<SecurityContext>,
    /// The answer plane ('lynx-ai'), when this deployment merges answers into
    /// responses. `None` — the default — renders the SERP only and dials nobody.
    pub answer: Option<Arc<AnswerClient>>,
    /// How many of the ranked head the answer plane may ground from
    /// (`ai.max_sources`).
    pub answer_max_sources: usize,
    /// Trust gate for the answer context (`ai.min_source_trust`): a candidate
    /// whose remaining trust (`1.0 - spam_score`) is below this is withheld
    /// from the head. `0.0` — the default — disables the gate.
    pub answer_min_trust: f64,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            reader: Arc::new(lynx_indexer::InMemoryIndex::new()),
            ranker: Ranker::default(),
            clock: None,
            started_at: OffsetDateTime::now_utc(),
            security: SecurityContext::disabled(),
            answer: None,
            answer_max_sources: 10,
            answer_min_trust: 0.0,
        }
    }
}

impl AppState {
    /// A state for structural tests that only need a valid construction, e.g. the
    /// router's middleware wiring. Not used by handlers.
    #[must_use]
    pub fn with_default_settings() -> Self {
        // Default settings for structural tests: rate limiting off, headers on.
        Self {
            security: crate::middleware::SecurityContext::disabled(),
            ..Self::default()
        }
    }
}

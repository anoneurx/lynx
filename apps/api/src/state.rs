//! Application state.

use std::sync::Arc;

use lynx_common::time::Clock;
use lynx_indexer::SearchIndexReader;
use lynx_ranker::Ranker;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct AppState {
    pub reader: Arc<dyn SearchIndexReader>,
    pub ranker: Ranker,
    pub clock: Option<Arc<dyn Clock>>,
    /// When the process started serving, for `/status` uptime.
    pub started_at: OffsetDateTime,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

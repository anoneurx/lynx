//! `lynx-api` — the public HTTP API.

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use lynx_ranker::Ranker;
use tracing::info;

use lynx_api::state::AppState;

#[tokio::main]
async fn main() {
    let opts = lynx_logging::Options::default();
    if let Err(error) = lynx_logging::init(&opts) {
        eprintln!("failed to initialise logging: {error}");
        return;
    }

    // The whole validated layered config: the answer plane (`ai.enabled` +
    // `ai.answer_service` dial the `lynx-ai` process) and the serving index
    // (`index.dir`) both come from here. Any problem is a boot failure, not a
    // per-request surprise — the same loader every plane uses.
    let config = match layered_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("lynx-api: {error}");
            return;
        }
    };
    let ai = config.ai.clone();
    let answer_client = if ai.enabled && !ai.answer_service.is_empty() {
        match lynx_api::answer::AnswerClient::new(&ai.answer_service, ai.answer_timeout_ms) {
            Ok(client) => Some(client),
            Err(error) => {
                eprintln!("lynx-api: cannot build the answer client: {error}");
                return;
            }
        }
    } else {
        None
    };

    // What we actually serve from: a sealed generation when one exists, the
    // development fixture when none does, and a refusal to boot in production.
    let serving = match lynx_api::reader::open(&config) {
        Ok(serving) => {
            info!(
                service = "lynx-api",
                "serving index: {}",
                serving.describe()
            );
            serving
        }
        Err(error) => {
            eprintln!("lynx-api: {error}");
            return;
        }
    };

    let app_state = Arc::new(AppState {
        reader: serving.reader(),
        ranker: Ranker::default(),
        clock: None,
        started_at: time::OffsetDateTime::now_utc(),
        security: lynx_api::middleware::SecurityContext::from_env(),
        answer: answer_client.as_ref().map(|c| Arc::new(c.clone())),
        answer_max_sources: usize::try_from(ai.max_sources).unwrap_or(10),
        answer_min_trust: ai.min_source_trust,
    });

    let app = lynx_api::router::router(app_state);

    let addr = std::env::var("LYNX_API_BIND")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8080)));
    info!(service = "lynx-api", addr = %addr, "starting");
    if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    } else {
        tracing::error!("failed to bind to {}", addr);
    }
}

/// The layered config (`config/example.toml` under
/// `config/$LYNX_PROFILE.toml`), validated. The answer merge is an opt-in
/// feature of the deployment, not of code paths: with `answer_service` empty
/// the API never dials anything.
///
/// # Errors
/// Missing/unreadable config files, parse errors, or environment-validation
/// issues, formatted for the boot log.
fn layered_config() -> Result<lynx_config::Config, String> {
    let base = std::fs::read_to_string("config/example.toml")
        .map_err(|e| format!("cannot read config/example.toml: {e}"))?;
    let profile = std::env::var("LYNX_PROFILE").unwrap_or_else(|_| "development".to_string());
    let layer = std::fs::read_to_string(format!("config/{profile}.toml"))
        .map_err(|e| format!("cannot read config/{profile}.toml: {e}"))?;
    let config = lynx_config::Config::from_toml_layers(&[base.as_str(), layer.as_str()])
        .map_err(|e| format!("invalid configuration: {e}"))?;
    if let Err(issues) = lynx_config::validate::validate(&config) {
        return Err(format!(
            "configuration rejected: {}",
            issues
                .iter()
                .map(|i| format!("{}: {}", i.field, i.message))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(config)
}

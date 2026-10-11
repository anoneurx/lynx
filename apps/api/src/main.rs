//! `lynx-api` — the public HTTP API.
//!
//! A thin shell: it wires configuration and logging, and will route search,
//! suggestions, and status requests to the services. The HTTP surface lands in a later
//! step; the binary currently starts, validates config, and exits cleanly.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let opts = lynx_logging::Options::default();
    if let Err(error) = lynx_logging::init(&opts) {
        eprintln!("failed to initialise logging: {error}");
        return ExitCode::FAILURE;
    }
    tracing::info!(
        service = lynx_query::SERVICE_NAME,
        version = env!("CARGO_PKG_VERSION"),
        "lynx-api starting"
    );
    ExitCode::SUCCESS
}

use tracing_subscriber::{EnvFilter, fmt};

mod handlers;
mod logging;
mod models;
mod server;
mod state;

/// Entry point that loads preferences, configures logging, and starts the server.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let prefs = models::Preferences::load().await;

    // Initialize file logging under preferences directory; fallback to stdout
    if let Err(e) = logging::init_logging(
        models::Preferences::log_dir(),
        &prefs.logging_level,
        &prefs.logging_format,
    ) {
        eprintln!("Logging init failed: {e}");
        let _ = fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .try_init();
    }

    let use_https = std::env::args().any(|arg| arg == "--https" || arg == "--tls");
    tracing::info!("Logging initialized; starting server (https={})", use_https);
    server::run(prefs, use_https).await
}

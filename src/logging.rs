use std::{fs, path::PathBuf, sync::OnceLock};
use tracing_appender::rolling::RollingFileAppender;
use tracing_subscriber::{EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, registry};

static WORKER_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// Initializes file-backed tracing with a daily rolling log in the preferences directory.
///
/// Falls back to the provided `level` when no `RUST_LOG` filter is set. The `_format`
/// parameter is currently retained for API compatibility.
pub fn init_logging(pref_path: PathBuf, level: &str, _format: &str) -> anyhow::Result<()> {
    let dir = pref_path;
    fs::create_dir_all(&dir)?;
    println!("Logging to {}", dir.display());
    let file_appender =
        RollingFileAppender::new(tracing_appender::rolling::Rotation::DAILY, dir, "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let _ = WORKER_GUARD.set(guard); // keep guard alive for program lifetime

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level.to_ascii_lowercase()));

    let subscriber = registry()
        .with(filter)
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false));

    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

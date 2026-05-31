use std::{fs::OpenOptions, path::Path, sync::OnceLock};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::LoggingConfig;

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn init_logging(config: &LoggingConfig) -> anyhow::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&config.file))?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    let _ = LOG_GUARD.set(guard);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.level));

    let stdout_layer = match config.format.as_str() {
        "json" => fmt::layer().json().with_writer(std::io::stdout).boxed(),
        _ => fmt::layer().with_writer(std::io::stdout).boxed(),
    };
    let file_layer = match config.format.as_str() {
        "json" => fmt::layer().json().with_writer(non_blocking).boxed(),
        _ => fmt::layer().with_writer(non_blocking).boxed(),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init()?;

    Ok(())
}

//! Structured logging to a local file (SPEC 15).
//!
//! Nothing is sent anywhere. The About box offers a "buka folder log" button,
//! and that folder is the only destination.

use std::path::{Path, PathBuf};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Keeps the background writer alive. Dropping it stops log flushing, so the
/// caller must hold it for the life of the process.
pub struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

impl std::fmt::Debug for LogGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogGuard")
    }
}

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Starts logging to `<data_dir>/logs/izul.log`, rotated daily.
pub fn init(data_dir: &Path) -> std::io::Result<LogGuard> {
    let dir = log_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let appender = tracing_appender::rolling::daily(&dir, "izul.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env("IZUL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(writer)
        .with_ansi(false)
        .with_current_span(true);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    // A debug build also prints to the terminal; a shipped build does not, so a
    // console window can never appear behind the application.
    #[cfg(debug_assertions)]
    let registry = registry.with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr));

    // `try_init` rather than `init`: the test harness may already have installed
    // a subscriber, and failing to log is not a reason to refuse to start.
    let _ = registry.try_init();
    Ok(LogGuard(guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_creates_its_directory_and_writes() {
        let dir = std::env::temp_dir().join(format!("izul-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let guard = init(&dir).expect("init logging");
        tracing::info!(marker = "phase0", "uji tulis log");
        drop(guard); // flushes

        let logs = log_dir(&dir);
        assert!(logs.is_dir(), "log directory must exist");
        let wrote_something = std::fs::read_dir(&logs)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.metadata().map(|m| m.len() > 0).unwrap_or(false))
            })
            .unwrap_or(false);
        assert!(
            wrote_something,
            "a log file with content must exist in {logs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

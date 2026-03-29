use anyhow::Result;
use chrono::Local;
use std::fmt as stdfmt;
use tracing_subscriber::fmt::{format::Writer, layer as fmt_layer, time::FormatTime};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> stdfmt::Result {
        write!(w, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

pub fn init_tracing(log_file: Option<&str>, verbose: bool) -> Result<()> {
    let level = if verbose { "debug" } else { "info" };

    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
    } else {
        EnvFilter::new(level)
    };

    if let Some(log_path) = log_file {
        let file_appender = tracing_appender::rolling::never(
            std::path::Path::new(log_path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
            std::path::Path::new(log_path)
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("export.log")),
        );

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt_layer()
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .with_timer(LocalTimer),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer().with_timer(LocalTimer))
            .init();
    }

    Ok(())
}

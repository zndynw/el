use anyhow::Result;
use chrono::Local;
use std::fmt as stdfmt;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormattedFields, layer as fmt_layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, field::Visit, layer::SubscriberExt, util::SubscriberInitExt};

struct LogFormatter {
    tag: Option<String>,
}

struct EventFieldVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl EventFieldVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }
}

impl Visit for EventFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!(r#"{}="{}""#, field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            if self.message.is_none() {
                self.message = Some(format!("{value:?}"));
            }
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}

impl<S, N> FormatEvent<S, N> for LogFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> stdfmt::Result {
        write!(
            writer,
            "{} {:>5}",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            event.metadata().level(),
        )?;

        if let Some(tag) = &self.tag {
            write!(writer, " [{}]", tag)?;
        }

        write!(writer, " {}", event.metadata().target())?;

        let mut visitor = EventFieldVisitor::new();
        event.record(&mut visitor);

        if let Some(message) = visitor.message {
            write!(writer, ": {}", message)?;
        } else {
            write!(writer, ":")?;
        }

        if !visitor.fields.is_empty() {
            write!(writer, " {}", visitor.fields.join(" "))?;
        }

        for span in ctx.event_scope().into_iter().flat_map(|scope| scope.from_root()) {
            let ext = span.extensions();
            if let Some(fields) = ext.get::<FormattedFields<N>>() {
                if !fields.is_empty() {
                    write!(writer, " {}", fields)?;
                }
            }
        }

        writeln!(writer)
    }
}

pub fn init_tracing(log_file: Option<&str>, tag: Option<&str>, verbose: bool) -> Result<()> {
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
                    .event_format(LogFormatter {
                        tag: tag.map(ToOwned::to_owned),
                    })
                    .with_writer(file_appender)
                    .with_ansi(false),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt_layer()
                    .event_format(LogFormatter {
                        tag: tag.map(ToOwned::to_owned),
                    }),
            )
            .init();
    }

    Ok(())
}

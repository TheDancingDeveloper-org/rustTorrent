use std::{io::LineWriter, path::Path};

use anyhow::Context;
use bytes::Bytes;
use librtbit_core::spawn_utils::spawn;
use tracing::debug_span;
use tracing_subscriber::{
    EnvFilter, Layer, filter::FilterExt, fmt, fmt::MakeWriter, prelude::*, registry::LookupSpan,
};

struct Subscriber {
    tx: tokio::sync::broadcast::Sender<Bytes>,
}

struct Writer {
    tx: tokio::sync::broadcast::Sender<Bytes>,
}

pub type LineBroadcast = tokio::sync::broadcast::Sender<Bytes>;

impl Subscriber {
    pub fn new() -> (Self, LineBroadcast) {
        let (tx, _) = tokio::sync::broadcast::channel(100);
        (Self { tx: tx.clone() }, tx)
    }
}

impl MakeWriter<'_> for Subscriber {
    type Writer = LineWriter<Writer>;

    fn make_writer(&self) -> Self::Writer {
        LineWriter::new(Writer {
            tx: self.tx.clone(),
        })
    }
}

impl std::io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = buf.len();
        if self.tx.receiver_count() == 0 {
            return Ok(len);
        }
        let arc = buf.to_vec().into();
        let _ = self.tx.send(arc);
        Ok(len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Feeds the in-memory [`crate::log_ring`] so recent history can be served
/// over the HTTP API.
struct RingLayer;

#[derive(Default)]
struct RingVisitor {
    message: String,
    fields: String,
}

impl tracing::field::Visit for RingVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = RingVisitor::default();
        event.record(&mut visitor);
        visitor.message.push_str(&visitor.fields);
        crate::log_ring::global().push((*event.metadata().level()).into(), visitor.message);
    }
}

/// Output format for the console and log-file layers. The HTTP API log broadcast is always JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

/// How often the log file is rolled over. `Never` keeps appending to one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogRotation {
    #[default]
    Never,
    Hourly,
    Daily,
}

#[derive(Default)]
pub struct InitLoggingOptions<'a> {
    pub default_rust_log_value: Option<&'a str>,
    pub log_file: Option<&'a str>,
    pub log_file_rust_log: Option<&'a str>,
    pub log_file_rotation: LogRotation,
    /// How many rolled files to keep. `None` keeps them all. Needs a rotation.
    pub log_file_max_files: Option<usize>,
    pub log_format: LogFormat,
}

pub struct InitLoggingResult {
    pub rust_log_reload_tx: tokio::sync::mpsc::UnboundedSender<String>,
    pub line_broadcast: LineBroadcast,
}

/// A text or JSON formatting layer, boxed so both formats share one type.
fn fmt_layer<S, W>(format: LogFormat, writer: W, ansi: bool) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = fmt::layer::<S>().with_ansi(ansi).with_writer(writer);
    match format {
        LogFormat::Text => layer.boxed(),
        LogFormat::Json => layer.json().boxed(),
    }
}

/// Open the log file, rolling it over when a rotation is set. A rolled file is named
/// `<file name>.<date>`, e.g. `rtbit.log.2026-10-17`.
fn open_log_file(
    path: &str,
    rotation: LogRotation,
    max_files: Option<usize>,
) -> anyhow::Result<tracing_appender::rolling::RollingFileAppender> {
    let path = Path::new(path);
    let directory = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("invalid log file path {path:?}"))?;

    let rotation = match rotation {
        LogRotation::Never => tracing_appender::rolling::Rotation::NEVER,
        LogRotation::Hourly => tracing_appender::rolling::Rotation::HOURLY,
        LogRotation::Daily => tracing_appender::rolling::Rotation::DAILY,
    };
    let mut builder = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(rotation)
        .filename_prefix(file_name);
    if let Some(max_files) = max_files {
        builder = builder.max_log_files(max_files);
    }
    builder
        .build(directory)
        .with_context(|| format!("error opening log file {:?}", path.display()))
}

#[inline(never)]
pub fn init_logging(opts: InitLoggingOptions) -> anyhow::Result<InitLoggingResult> {
    if opts.log_file.is_none()
        && (opts.log_file_rotation != LogRotation::Never || opts.log_file_max_files.is_some())
    {
        anyhow::bail!("log file rotation needs a log file (--log-file)");
    }
    if opts.log_file_max_files.is_some() && opts.log_file_rotation == LogRotation::Never {
        anyhow::bail!("--log-file-max-files needs --log-file-rotation hourly or daily");
    }

    let stderr_filter = EnvFilter::builder()
        .with_default_directive(
            opts.default_rust_log_value
                .unwrap_or("info")
                .parse()
                .context("can't parse provided rust_log value")?,
        )
        .from_env()
        .context("invalid RUST_LOG value")?;

    let (stderr_filter, reload_stderr_filter) =
        tracing_subscriber::reload::Layer::new(stderr_filter);

    let (line_sub, line_broadcast) = Subscriber::new();

    let file_layer = match opts.log_file {
        Some(log_file) => {
            let appender =
                open_log_file(log_file, opts.log_file_rotation, opts.log_file_max_files)?;
            Some(
                fmt_layer(opts.log_format, appender, false).with_filter(
                    EnvFilter::builder()
                        .parse(opts.log_file_rust_log.unwrap_or("info,librtbit=debug"))
                        .context("can't parse log-file-rust-log")?,
                ),
            )
        }
        None => None,
    };

    let layered = tracing_subscriber::registry()
        // Log file layer, if one was requested.
        .with(file_layer)
        // Console logging layer.
        .with(fmt_layer(opts.log_format, std::io::stdout, true).with_filter(stderr_filter))
        // Retained log history for the HTTP API.
        .with(RingLayer.with_filter(EnvFilter::builder().parse("info").unwrap()))
        // HTTP API log broadcast layer.
        .with(
            fmt::layer()
                .with_ansi(false)
                .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new())
                .event_format(fmt::format().with_ansi(false).json())
                .with_writer(line_sub)
                .with_filter(
                    tracing_subscriber::filter::filter_fn({
                        let line_broadcast = line_broadcast.clone();
                        move |_| line_broadcast.receiver_count() > 0
                    })
                    .and(EnvFilter::builder().parse("info,librtbit=debug").unwrap()),
                ),
        );

    #[cfg(feature = "tokio-console")]
    let console_layer = console_subscriber::spawn();

    #[cfg(feature = "tokio-console")]
    let layered = layered.with(console_layer);

    layered.try_init().context("can't init logging")?;

    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    spawn::<&'static str>(
        debug_span!("fmt_filter_reloader"),
        "fmt_filter_reloader",
        async move {
            while let Some(rust_log) = reload_rx.recv().await {
                let stderr_env_filter = match EnvFilter::builder().parse(&rust_log) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("can't parse env filter {rust_log:?}: {e:#?}");
                        continue;
                    }
                };
                eprintln!("setting RUST_LOG to {rust_log:?}");
                let _ = reload_stderr_filter.reload(stderr_env_filter);
            }
            Ok(())
        },
    );
    Ok(InitLoggingResult {
        rust_log_reload_tx: reload_tx,
        line_broadcast,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{LogRotation, open_log_file};

    #[test]
    fn never_rotation_appends_to_the_named_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rtbit.log");
        let mut appender = open_log_file(path.to_str().unwrap(), LogRotation::Never, None).unwrap();
        writeln!(appender, "first").unwrap();
        drop(appender);

        let mut appender = open_log_file(path.to_str().unwrap(), LogRotation::Never, None).unwrap();
        writeln!(appender, "second").unwrap();
        drop(appender);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn daily_rotation_writes_a_dated_file_next_to_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rtbit.log");
        let mut appender =
            open_log_file(path.to_str().unwrap(), LogRotation::Daily, Some(3)).unwrap();
        writeln!(appender, "entry").unwrap();
        drop(appender);

        assert!(
            !path.exists(),
            "the bare name is not written under rotation"
        );
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("rtbit.log."), "got {names:?}");
    }

    /// Collects formatted log output in memory.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn log_one_event(format: super::LogFormat) -> String {
        use tracing_subscriber::prelude::*;

        let captured = Captured::default();
        let subscriber =
            tracing_subscriber::registry().with(super::fmt_layer(format, captured.clone(), false));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(torrent_id = 7, "added torrent");
        });
        let bytes = captured.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn json_format_writes_one_json_object_per_event() {
        let output = log_one_event(super::LogFormat::Json);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 1, "output: {output:?}");
        let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event["level"], "INFO");
        assert_eq!(event["fields"]["message"], "added torrent");
        assert_eq!(event["fields"]["torrent_id"], 7);
        assert!(event["timestamp"].is_string());
    }

    #[test]
    fn text_format_is_plain_text_without_ansi() {
        let output = log_one_event(super::LogFormat::Text);
        assert!(output.contains("INFO"), "output: {output:?}");
        assert!(
            output.contains("added torrent torrent_id=7"),
            "output: {output:?}"
        );
        assert!(serde_json::from_str::<serde_json::Value>(output.trim()).is_err());
        assert!(!output.contains('\x1b'), "ANSI escapes with ansi=false");
    }
}

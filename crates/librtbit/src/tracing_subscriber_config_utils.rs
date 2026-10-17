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
}

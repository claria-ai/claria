//! Shared logging configuration: which crates count as Claria's own, the
//! filter strings built from that one list, and the on-disk log location.

use std::path::PathBuf;

/// Every workspace crate, as a tracing target prefix. The console, terminal,
/// and file filters are all built from this one list so a new crate cannot be
/// silently missing from one of them.
pub const CLARIA_CRATES: &[&str] = &[
    "claria_bedrock",
    "claria_billing",
    "claria_core",
    "claria_desktop",
    "claria_docx",
    "claria_provisioner",
    "claria_records",
    "claria_report_pipeline",
    "claria_report_store",
    "claria_storage",
    "claria_transcribe",
];

/// An `EnvFilter` directive string admitting trace-level events from every
/// Claria crate on top of `base` (e.g. `"info"`), while SDK/hyper trace noise
/// stays at the base level.
pub fn claria_trace_filter(base: &str) -> String {
    std::iter::once(base.to_string())
        .chain(CLARIA_CRATES.iter().map(|krate| format!("{krate}=trace")))
        .collect::<Vec<_>>()
        .join(",")
}

/// The platform's application log directory for Claria, mirroring Tauri's
/// `app_log_dir()` conventions. Computed without an app handle so the tracing
/// stack can be wired up before Tauri starts.
pub fn app_log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library/Logs/com.claria.desktop"))
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir().map(|dir| dir.join("com.claria.desktop").join("logs"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs::data_dir().map(|dir| dir.join("com.claria.desktop").join("logs"))
    }
}

/// How many daily log files to keep before the oldest is deleted.
pub const MAX_LOG_FILES: usize = 5;

/// Build the rolling file appender under [`app_log_dir`], creating the
/// directory. Returns `None` (after printing why) when the platform has no
/// log directory or the appender cannot be built — logging to a file is
/// never worth failing startup over.
pub fn rolling_file_appender() -> Option<tracing_appender::rolling::RollingFileAppender> {
    let Some(dir) = app_log_dir() else {
        eprintln!("no application log directory on this platform; file logging disabled");
        return None;
    };
    match tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("claria")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(&dir)
    {
        Ok(appender) => Some(appender),
        Err(error) => {
            eprintln!(
                "failed to initialize file logging in {}: {error}",
                dir.display()
            );
            None
        }
    }
}

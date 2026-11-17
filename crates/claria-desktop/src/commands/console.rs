//! Claria Console log commands.

use serde::{Deserialize, Serialize};
use tauri::State;

use claria_desktop::console::{ConsoleBuffer, ConsoleDelta};

use super::{CommandError, flatten};

/// Severity levels the frontend logging bridge may report.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FrontendLogLevel {
    Error,
    Warn,
    Info,
}

/// Record a frontend-reported event into the shared tracing stack.
///
/// The `claria_desktop::frontend` target is admitted by the console and file
/// layers via the shared crate-list filter, so webview failures land in the
/// ring buffer and the rolling on-disk logs. The webview can pass arbitrary
/// error text (including a provider echo of PHI); only a fixed event family,
/// byte count, and fingerprint enter the logs.
#[tauri::command]
#[specta::specta]
pub fn log_frontend_event(level: FrontendLogLevel, message: String) {
    let (event_family, bytes, message_sha256) =
        claria_desktop::console::frontend_event_fields(&message);
    match level {
        FrontendLogLevel::Error => {
            tracing::error!(target: "claria_desktop::frontend", event_family, bytes, message_sha256, "frontend event");
        }
        FrontendLogLevel::Warn => {
            tracing::warn!(target: "claria_desktop::frontend", event_family, bytes, message_sha256, "frontend event");
        }
        FrontendLogLevel::Info => {
            tracing::info!(target: "claria_desktop::frontend", event_family, bytes, message_sha256, "frontend event");
        }
    }
}

/// Console entries at or after the sequence cursor `seq`. Pass the previous
/// response's `next_seq`; start (or force a full refetch) with `0`.
#[tauri::command]
#[specta::specta]
pub fn get_console_logs_since(console: State<'_, ConsoleBuffer>, seq: u64) -> ConsoleDelta {
    console.entries_since(seq)
}

#[tauri::command]
#[specta::specta]
pub fn get_console_logs_text(console: State<'_, ConsoleBuffer>) -> String {
    console.to_text()
}

#[tauri::command]
#[specta::specta]
pub fn save_console_logs(console: State<'_, ConsoleBuffer>) -> Result<bool, String> {
    flatten("save_console_logs", save_console_logs_inner(&console))
}

fn save_console_logs_inner(console: &ConsoleBuffer) -> Result<bool, CommandError> {
    let text = console.to_text();
    let date = jiff::Timestamp::now().strftime("%Y-%m-%d").to_string();

    let path = rfd::FileDialog::new()
        .set_file_name(format!("claria-console-{date}.log"))
        .add_filter("Log files", &["log", "txt"])
        .save_file();

    match path {
        Some(p) => {
            // The console export can carry sensitive operational detail, so it
            // gets the same private-atomic write as every user-exported file.
            claria_desktop::local_export::write_private_atomic(&p, text.as_bytes())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

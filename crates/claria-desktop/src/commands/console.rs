//! Claria Console log commands.

use tauri::State;

use claria_desktop::console::{ConsoleBuffer, ConsoleEntry};

use super::{CommandError, flatten};

#[tauri::command]
#[specta::specta]
pub fn get_console_logs(console: State<'_, ConsoleBuffer>) -> Vec<ConsoleEntry> {
    console.entries()
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
            std::fs::write(&p, text).map_err(|e| CommandError::Msg(e.to_string()))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

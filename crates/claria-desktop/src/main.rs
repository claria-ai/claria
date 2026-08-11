#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eyre::Result;
use tauri::{
    Manager,
    menu::{HELP_SUBMENU_ID, Menu, MenuItem, MenuItemKind, PredefinedMenuItem},
    webview::WebviewWindowBuilder,
};
use tauri_specta::{Builder, collect_commands};
use tracing_subscriber::prelude::*;

use claria_desktop::console;

mod commands;
mod local_transcription;
mod report_template_commands;
mod state;

fn main() -> Result<()> {
    color_eyre::install()?;

    let console_buffer = console::ConsoleBuffer::new();
    let console_layer = console::ConsoleLayer::new(console_buffer.clone());

    // Per-layer filters, not a global one: a global filter runs before the
    // layers and would drop the trace-level timing spans everywhere, including
    // the export. The terminal honors RUST_LOG (default "info"), so timing spans
    // stay out of the normal operational log; RUST_LOG=claria_storage=trace (etc.)
    // still surfaces them there for ad-hoc debugging.
    let fmt_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // The exported console log always admits the claria_* timing spans on top of
    // the info baseline, so a log a user sends in carries durations with zero
    // configuration, while SDK/hyper trace noise stays out. The directive list
    // is built from the one shared crate list in `logging`.
    let console_filter =
        tracing_subscriber::EnvFilter::new(claria_desktop::logging::claria_trace_filter("info"));

    // A rolling on-disk log with the same filter as the console layer, so a
    // support request can be answered from files that survive an app crash or
    // restart. Daily rotation, bounded file count, PHI-scrubbed like every
    // other layer.
    let file_layer = claria_desktop::logging::rolling_file_appender().map(|appender| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .with_writer(appender)
            .with_filter(tracing_subscriber::EnvFilter::new(
                claria_desktop::logging::claria_trace_filter("info"),
            ))
    });

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_filter(fmt_filter),
        )
        .with(console_layer.with_filter(console_filter))
        .with(file_layer)
        .init();

    let builder = Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::has_config,
        commands::load_config,
        commands::save_config,
        commands::delete_config,
        commands::save_preferences_patch,
        commands::fetch_cloud_preferences,
        commands::upload_record_file_with_options,
        commands::save_transcript_edits,
        commands::pick_audio_file,
        commands::set_preferred_model,
        commands::assess_credentials,
        commands::assume_role,
        commands::list_aws_profiles,
        commands::list_user_access_keys,
        commands::delete_user_access_key,
        commands::bootstrap_iam_user,
        commands::escalate_iam_policy,
        commands::provision_scan,
        commands::provision_apply,
        commands::plan,
        commands::apply,
        commands::destroy,
        commands::reset_provisioner_state,
        commands::list_clients,
        commands::create_client,
        commands::get_client_record_details,
        commands::update_client_name,
        commands::delete_client,
        commands::load_report_workspace,
        commands::list_editor_history,
        commands::rename_report_session,
        commands::list_report_revisions,
        commands::load_report_revision,
        commands::revert_report_revision,
        commands::save_report_draft,
        report_template_commands::list_writer_templates,
        report_template_commands::upload_writer_template,
        report_template_commands::rename_writer_template,
        report_template_commands::delete_writer_template,
        report_template_commands::preview_writer_template,
        report_template_commands::apply_report_template,
        report_template_commands::discard_report_template_preview,
        commands::generate_full_report,
        commands::send_report_message,
        commands::resolve_report_proposal,
        commands::export_report_docx,
        commands::list_record_files,
        commands::search_record_contents,
        commands::upload_record_file,
        commands::delete_record_file,
        commands::get_record_file_text,
        commands::create_text_record_file,
        commands::update_text_record_file,
        commands::list_record_context,
        commands::extract_record_file,
        commands::list_chat_models,
        commands::chat_message,
        commands::infra_chat,
        commands::accept_model_agreement,
        commands::list_chat_histories,
        commands::load_chat_history,
        commands::rename_chat_history,
        commands::get_prompt,
        commands::save_prompt,
        commands::delete_prompt,
        commands::list_prompt_versions,
        commands::get_prompt_version,
        commands::restore_prompt_version,
        commands::list_file_versions,
        commands::get_file_version_text,
        commands::restore_file_version,
        commands::list_deleted_files,
        commands::restore_deleted_file,
        commands::list_deleted_clients,
        commands::restore_client,
        local_transcription::get_local_transcription_status,
        local_transcription::save_local_transcription_settings,
        local_transcription::download_local_model,
        local_transcription::delete_local_model,
        local_transcription::delete_legacy_transcription_models,
        local_transcription::transcribe_memo,
        commands::check_for_updates,
        commands::get_cost_and_usage,
        commands::probe_cost_explorer,
        commands::enable_cost_explorer,
        commands::set_hourly_cost_data,
        commands::lookup_model_pricing,
        commands::open_url,
        commands::reveal_log_folder,
        commands::count_client_context_tokens,
        commands::count_infra_context_tokens,
        commands::get_console_logs_since,
        commands::get_console_logs_text,
        commands::save_console_logs,
        commands::log_frontend_event,
    ]);

    #[cfg(debug_assertions)]
    {
        let bindings_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../claria-desktop-frontend/src/lib/bindings.ts");
        // `TurnUsage.input_tokens: u64` (and similar) round-trip cleanly as
        // JS `number` for token counts and pricing values we expect — bump
        // `bigint` to `Number` so specta accepts u64 fields.
        let ts_config = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number);
        builder
            .export(ts_config, &bindings_path)
            .expect("failed to export typescript bindings");

        // Prepend // @ts-nocheck so the generated file passes strict TypeScript
        // linting (specta emits some unused imports/functions).
        let contents =
            std::fs::read_to_string(&bindings_path).expect("failed to read generated bindings");
        std::fs::write(&bindings_path, format!("// @ts-nocheck\n{contents}"))
            .expect("failed to write @ts-nocheck header");
    }

    tauri::Builder::default()
        .manage(state::DesktopState::default())
        .manage(console_buffer)
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Keep Tauri's native application menu—including the standard
            // Edit actions that make Cmd/Ctrl+C/V/X/A work in the webview—and
            // add Claria Console to its Help submenu. Replacing the whole menu
            // with Help alone silently disables normal clipboard shortcuts.
            let console_item =
                MenuItem::with_id(app, "console", "Claria Console", true, None::<&str>)?;
            let menu = Menu::default(app.handle())?;
            if let Some(MenuItemKind::Submenu(help_menu)) = menu.get(HELP_SUBMENU_ID) {
                help_menu.append(&PredefinedMenuItem::separator(app)?)?;
                help_menu.append(&console_item)?;
            }
            app.set_menu(menu)?;

            app.on_menu_event(move |app, event| {
                if event.id() == "console" {
                    // Focus existing console window or create a new one.
                    if let Some(win) = app.get_webview_window("console") {
                        let _ = win.set_focus();
                    } else {
                        let _ = WebviewWindowBuilder::new(
                            app,
                            "console",
                            tauri::WebviewUrl::App("index.html#console".into()),
                        )
                        .title("Claria Console")
                        .inner_size(900.0, 600.0)
                        .build();
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| eyre::eyre!("tauri error: {e}"))?;

    Ok(())
}

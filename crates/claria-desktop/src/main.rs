#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eyre::Result;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::webview::WebviewWindowBuilder;
use tauri::Manager;
use tauri_specta::{collect_commands, Builder};
use tracing_subscriber::prelude::*;

mod commands;
mod console;
mod state;

fn main() -> Result<()> {
    color_eyre::install()?;

    let console_buffer = console::ConsoleBuffer::new();
    let console_layer = console::ConsoleLayer::new(console_buffer.clone());

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(console_layer)
        .init();

    let builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::has_config,
            commands::load_config,
            commands::save_config,
            commands::delete_config,
            commands::set_preferred_model,
            commands::assess_credentials,
            commands::assume_role,
            commands::list_aws_profiles,
            commands::list_user_access_keys,
            commands::delete_user_access_key,
            commands::bootstrap_iam_user,
            commands::escalate_iam_policy,
            commands::plan,
            commands::apply,
            commands::destroy,
            commands::reset_provisioner_state,
            commands::list_clients,
            commands::create_client,
            commands::delete_client,
            commands::list_record_files,
            commands::upload_record_file,
            commands::delete_record_file,
            commands::get_record_file_text,
            commands::create_text_record_file,
            commands::update_text_record_file,
            commands::list_record_context,
            commands::list_chat_models,
            commands::chat_message,
            commands::infra_chat,
            commands::accept_model_agreement,
            commands::load_chat_history,
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
            commands::get_whisper_models,
            commands::download_whisper_model,
            commands::delete_whisper_model,
            commands::delete_whisper_model_dir,
            commands::set_active_whisper_model,
            commands::transcribe_memo,
            commands::check_for_updates,
            commands::get_cost_and_usage,
            commands::probe_cost_explorer,
            commands::enable_cost_explorer,
            commands::set_hourly_cost_data,
            commands::open_url,
            commands::count_client_context_tokens,
            commands::count_infra_context_tokens,
            commands::get_console_logs,
            commands::get_console_logs_text,
        ]);

    #[cfg(debug_assertions)]
    {
        let bindings_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../claria-desktop-frontend/src/lib/bindings.ts");
        builder
            .export(specta_typescript::Typescript::default(), &bindings_path)
            .expect("failed to export typescript bindings");

        // Prepend // @ts-nocheck so the generated file passes strict TypeScript
        // linting (specta emits some unused imports/functions).
        let contents = std::fs::read_to_string(&bindings_path)
            .expect("failed to read generated bindings");
        std::fs::write(&bindings_path, format!("// @ts-nocheck\n{contents}"))
            .expect("failed to write @ts-nocheck header");
    }

    tauri::Builder::default()
        .manage(state::DesktopState::default())
        .manage(console_buffer)
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Build native Help menu with "Claria Console" item.
            let console_item =
                MenuItem::with_id(app, "console", "Claria Console", true, None::<&str>)?;
            let help_menu =
                Submenu::with_items(app, "Help", true, &[&console_item])?;
            let menu = Menu::with_items(app, &[&help_menu])?;
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
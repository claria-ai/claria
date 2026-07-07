#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eyre::Result;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::webview::WebviewWindowBuilder;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};
use tracing_subscriber::prelude::*;

use claria_desktop::console;

mod commands;
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
    // configuration, while SDK/hyper trace noise stays out.
    let console_filter = tracing_subscriber::EnvFilter::new(
        "info,claria_storage=trace,claria_bedrock=trace,claria_desktop=trace",
    );

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_filter(fmt_filter),
        )
        .with(console_layer.with_filter(console_filter))
        .init();

    let builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::has_config,
            commands::load_config,
            commands::save_config,
            commands::delete_config,
            commands::save_preferences,
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
            commands::delete_client,
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
            commands::lookup_model_pricing,
            commands::open_url,
            commands::count_client_context_tokens,
            commands::count_infra_context_tokens,
            commands::get_console_logs,
            commands::get_console_logs_text,
            commands::save_console_logs,
            commands::get_lock_state,
            commands::record_activity,
            commands::lock_session,
            commands::unlock_with_pin,
            commands::unlock_with_biometric,
            commands::get_biometry_status,
            commands::enable_auto_lock,
            commands::disable_auto_lock,
            commands::change_pin,
            commands::set_auto_lock_timeout,
            commands::set_biometric_unlock,
        ])
        .events(collect_events![commands::LockStateChanged]);

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
        let contents = std::fs::read_to_string(&bindings_path)
            .expect("failed to read generated bindings");
        std::fs::write(&bindings_path, format!("// @ts-nocheck\n{contents}"))
            .expect("failed to write @ts-nocheck header");
    }

    tauri::Builder::default()
        .manage(state::DesktopState::default())
        .manage(console_buffer)
        .plugin(tauri_plugin_biometry::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Start locked when auto-lock is configured, before any window
            // can render PHI. The frontend LockGate reads this via
            // `get_lock_state` on mount.
            if claria_desktop::config::has_config() {
                match claria_desktop::config::load_config() {
                    Ok(cfg)
                        if cfg.security.auto_lock_enabled && cfg.security.pin_hash.is_some() =>
                    {
                        let state = app.state::<state::DesktopState>();
                        if let Ok(mut rt) = state.lock.lock() {
                            rt.lock();
                        }
                        commands::emit_session_audit(
                            "session_lock",
                            Some(&cfg),
                            serde_json::json!({ "reason": "startup" }),
                        );
                    }
                    _ => {}
                }
            }

            // Idle/sleep watcher. Idle expiry comes from the frontend's
            // throttled activity reports; a wall-clock jump past one tick
            // means the machine slept, so lock on wake.
            let watcher_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::time::Duration;
                const TICK: Duration = Duration::from_secs(10);
                const SLEEP_SLACK: Duration = Duration::from_secs(60);

                let mut prev_wall = jiff::Timestamp::now();
                loop {
                    tokio::time::sleep(TICK).await;
                    let now_wall = jiff::Timestamp::now();
                    let jumped = claria_desktop::security::wall_clock_jumped(
                        prev_wall,
                        now_wall,
                        TICK,
                        SLEEP_SLACK,
                    );
                    prev_wall = now_wall;

                    let state = watcher_app.state::<state::DesktopState>();
                    let (armed, timeout) = {
                        let guard = state.config.lock().await;
                        match guard.as_ref() {
                            Some(c) if c.security.auto_lock_enabled
                                && c.security.pin_hash.is_some() =>
                            {
                                let mins = u64::from(c.security.auto_lock_timeout_minutes);
                                (true, Duration::from_secs(mins * 60))
                            }
                            _ => (false, Duration::ZERO),
                        }
                    };
                    if !armed {
                        continue;
                    }

                    let idle = state
                        .lock
                        .lock()
                        .map(|rt| rt.idle_expired(timeout))
                        .unwrap_or(false);

                    let reason = if jumped {
                        Some("sleep")
                    } else if idle {
                        Some("idle")
                    } else {
                        None
                    };
                    if let Some(reason) = reason
                        && let Err(e) = commands::engage_lock(&watcher_app, reason).await
                    {
                        tracing::warn!("auto-lock failed: {e}");
                    }
                }
            });

            // Build native Help menu with "Claria Console" and "Lock Claria".
            let console_item =
                MenuItem::with_id(app, "console", "Claria Console", true, None::<&str>)?;
            let lock_item =
                MenuItem::with_id(app, "lock", "Lock Claria", true, None::<&str>)?;
            let help_menu =
                Submenu::with_items(app, "Help", true, &[&console_item, &lock_item])?;
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
                } else if event.id() == "lock" {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = commands::engage_lock(&app, "manual").await {
                            tracing::warn!("manual lock failed: {e}");
                        }
                    });
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| eyre::eyre!("tauri error: {e}"))?;

    Ok(())
}
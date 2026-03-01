#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eyre::Result;
use tauri_specta::{collect_commands, Builder};

mod commands;
mod state;

fn main() -> Result<()> {
    color_eyre::install()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::has_config,
            commands::load_config,
            commands::save_config,
            commands::delete_config,
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
            commands::accept_model_agreement,
            commands::load_chat_history,
            commands::get_system_prompt,
            commands::save_system_prompt,
            commands::delete_system_prompt,
            commands::list_file_versions,
            commands::get_file_version_text,
            commands::restore_file_version,
            commands::list_deleted_files,
            commands::restore_deleted_file,
            commands::list_deleted_clients,
            commands::restore_client,
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
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| eyre::eyre!("tauri error: {e}"))?;

    Ok(())
}
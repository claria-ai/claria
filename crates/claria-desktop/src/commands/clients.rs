//! Client record CRUD commands, backed by S3.

use tauri::State;

pub use claria_records::{ClientNameUpdate, ClientRecordDetails, ClientSummary};

use super::{CommandContext, parse_uuid, run};
use crate::state::DesktopState;

/// List all client records from S3.
///
/// Loads each `clients/{id}.json` object, deserializes the Client, and
/// returns summaries sorted by most recently created first.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(count = tracing::field::Empty))]
pub async fn list_clients(state: State<'_, DesktopState>) -> Result<Vec<ClientSummary>, String> {
    run("list_clients", async {
        let ctx = CommandContext::new(&state).await?;

        let clients =
            claria_records::list_client_summaries(&ctx.s3, &ctx.bucket, &state.record_cache)
                .await?;

        tracing::Span::current().record("count", clients.len() as u64);

        Ok(clients)
    })
    .await
}

/// Create a new client record in S3.
#[tauri::command]
#[specta::specta]
pub async fn create_client(
    state: State<'_, DesktopState>,
    name: String,
) -> Result<ClientSummary, String> {
    run("create_client", async {
        let name = claria_records::validate_client_name(&name)?;
        let ctx = CommandContext::new(&state).await?;

        let id = uuid::Uuid::new_v4();
        let now = jiff::Timestamp::now();
        let client = claria_core::models::client::Client {
            id,
            name: name.clone(),
            created_at: now,
            updated_at: now,
        };

        let body = serde_json::to_vec_pretty(&client)?;
        let key = claria_core::s3_keys::client(id);

        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, body, Some("application/json"))
            .await?;

        // The client's name is PHI and never logged.
        tracing::info!(client_id = %id, "client record created");

        Ok(ClientSummary {
            id: id.to_string(),
            name,
            created_at: now.to_string(),
        })
    })
    .await
}

/// Load editable metadata, storage statistics, and name history for one client.
#[tauri::command]
#[specta::specta]
pub async fn get_client_record_details(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<ClientRecordDetails, String> {
    run("get_client_record_details", async {
        let ctx = CommandContext::new(&state).await?;
        let id = parse_uuid(&client_id)?;
        Ok(claria_records::get_client_record_details(&ctx.s3, &ctx.bucket, id).await?)
    })
    .await
}

/// Update a client's display name with optimistic concurrency control.
#[tauri::command]
#[specta::specta]
pub async fn update_client_name(
    state: State<'_, DesktopState>,
    client_id: String,
    name: String,
) -> Result<ClientNameUpdate, String> {
    run("update_client_name", async {
        let ctx = CommandContext::new(&state).await?;
        let id = parse_uuid(&client_id)?;
        let update = claria_records::update_client_name(&ctx.s3, &ctx.bucket, id, &name).await?;
        tracing::info!(client_id = %id, "client record renamed");
        Ok(update)
    })
    .await
}

/// Delete a client and all associated data through the retryable,
/// compensating lifecycle library.
#[tauri::command]
#[specta::specta]
pub async fn delete_client(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<(), String> {
    run("delete_client", async {
        let ctx = CommandContext::new(&state).await?;
        let id = parse_uuid(&client_id)?;
        let outcome = claria_records::delete_client(&ctx.s3, &ctx.bucket, id).await?;
        tracing::info!(
            client_id = %id,
            deleted_records = outcome.deleted_records,
            deleted_report_objects = outcome.deleted_report_objects,
            "client deleted"
        );
        Ok(())
    })
    .await
}

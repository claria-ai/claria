//! Retryable, compensating lifecycle operations for client-owned S3 data.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::client::Client;
use claria_storage::error::StorageError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const LIFECYCLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ClientLifecycleError {
    #[error("The client record does not match its storage key.")]
    InvalidClient,

    #[error("The client deletion recovery state is invalid.")]
    InvalidRecoveryState,

    #[error("Client storage is unavailable while {operation}.")]
    Storage {
        operation: &'static str,
        #[source]
        source: StorageError,
    },

    #[error("The report workspace could not be restored safely.")]
    ReportRestore {
        #[source]
        source: claria_report_authoring::ReportAuthoringError,
    },

    #[error("Client deletion failed and was rolled back safely.")]
    DeletionRolledBack {
        #[source]
        source: Box<ClientLifecycleError>,
    },

    #[error(
        "Client deletion stopped and automatic recovery is incomplete. Retry the deletion before editing this client."
    )]
    RecoveryIncomplete {
        #[source]
        source: Box<ClientLifecycleError>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientDeletionOutcome {
    pub deleted_records: usize,
    pub deleted_report_objects: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientRestoreOutcome {
    pub report_restored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientDeletionState {
    schema_version: u32,
    client_id: Uuid,
    phase: ClientDeletionPhase,
    started_at: jiff::Timestamp,
    updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClientDeletionPhase {
    Started,
    ReportDeleted,
    RecordsDeleted,
}

pub async fn delete_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ClientDeletionOutcome, ClientLifecycleError> {
    let lifecycle_key = claria_core::s3_keys::client_lifecycle(client_id);
    let client_key = claria_core::s3_keys::client(client_id);
    let records_prefix = claria_core::s3_keys::client_records_prefix(client_id);
    let report_prefix = claria_core::s3_keys::report_authoring_client_prefix(client_id);
    let mut lifecycle =
        load_or_start_deletion(s3, bucket, client_id, &client_key, &lifecycle_key).await?;

    let deletion_result = async {
        let deleted_report_objects =
            claria_storage::objects::delete_objects_by_prefix(s3, bucket, &report_prefix)
                .await
                .map_err(|source| storage("deleting report data", source))?;
        lifecycle.phase = ClientDeletionPhase::ReportDeleted;
        lifecycle.updated_at = jiff::Timestamp::now();
        claria_storage::state::save_state(s3, bucket, &lifecycle_key, &lifecycle)
            .await
            .map_err(|source| storage("recording report deletion", source))?;

        let deleted_records =
            claria_storage::objects::delete_objects_by_prefix(s3, bucket, &records_prefix)
                .await
                .map_err(|source| storage("deleting record data", source))?;
        lifecycle.phase = ClientDeletionPhase::RecordsDeleted;
        lifecycle.updated_at = jiff::Timestamp::now();
        claria_storage::state::save_state(s3, bucket, &lifecycle_key, &lifecycle)
            .await
            .map_err(|source| storage("recording record deletion", source))?;

        claria_storage::objects::delete_object(s3, bucket, &client_key)
            .await
            .map_err(|source| storage("deleting the client", source))?;
        Ok(ClientDeletionOutcome {
            deleted_records,
            deleted_report_objects,
        })
    }
    .await;

    match deletion_result {
        Ok(outcome) => {
            claria_storage::objects::delete_object(s3, bucket, &lifecycle_key)
                .await
                .map_err(|source| storage("finishing client deletion", source))?;
            Ok(outcome)
        }
        Err(original_error) => {
            let client_restore =
                claria_storage::objects::restore_deleted_object_if_absent(s3, bucket, &client_key)
                    .await;
            let records_restore = claria_storage::objects::restore_deleted_objects_by_prefix(
                s3,
                bucket,
                &records_prefix,
            )
            .await;
            let report_restore = claria_storage::objects::restore_deleted_objects_by_prefix(
                s3,
                bucket,
                &report_prefix,
            )
            .await;
            if client_restore.is_ok() && records_restore.is_ok() && report_restore.is_ok() {
                match delete_if_exists(s3, bucket, &lifecycle_key).await {
                    Ok(()) => Err(ClientLifecycleError::DeletionRolledBack {
                        source: Box::new(original_error),
                    }),
                    Err(_) => Err(ClientLifecycleError::RecoveryIncomplete {
                        source: Box::new(original_error),
                    }),
                }
            } else {
                Err(ClientLifecycleError::RecoveryIncomplete {
                    source: Box::new(original_error),
                })
            }
        }
    }
}

pub async fn restore_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ClientRestoreOutcome, ClientLifecycleError> {
    let client_key = claria_core::s3_keys::client(client_id);
    let versions = claria_storage::objects::list_object_versions(s3, bucket, &client_key)
        .await
        .map_err(|source| storage("listing client versions", source))?;
    let real = versions
        .iter()
        .find(|version| !version.is_delete_marker)
        .ok_or(ClientLifecycleError::InvalidClient)?;
    let output =
        claria_storage::objects::get_object_version(s3, bucket, &client_key, &real.version_id)
            .await
            .map_err(|source| storage("reading a client version", source))?;
    validate_client_body(&output.body, client_id)?;

    match claria_storage::objects::put_object_if_none_match(
        s3,
        bucket,
        &client_key,
        output.body,
        output.content_type.as_deref(),
    )
    .await
    {
        Ok(_) | Err(StorageError::PreconditionFailed { .. }) => {}
        Err(source) => return Err(storage("restoring the client", source)),
    }
    // A concurrent winner must still be the expected client.
    let current = claria_storage::objects::get_object(s3, bucket, &client_key)
        .await
        .map_err(|source| storage("validating the restored client", source))?;
    validate_client_body(&current.body, client_id)?;

    // Preserve the established restore behavior for record files and the
    // text-only Chat history stored beneath the same prefix: a successful
    // client deletion keeps those objects deleted after the client is
    // restored. Only the separate, opt-in report workspace follows the
    // restored client.
    let report_restored =
        claria_report_authoring::restore_report_workspace_for_client(s3, bucket, client_id)
            .await
            .map_err(|source| ClientLifecycleError::ReportRestore { source })?;
    delete_if_exists(
        s3,
        bucket,
        &claria_core::s3_keys::client_lifecycle(client_id),
    )
    .await?;

    Ok(ClientRestoreOutcome { report_restored })
}

async fn load_or_start_deletion(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    client_key: &str,
    lifecycle_key: &str,
) -> Result<ClientDeletionState, ClientLifecycleError> {
    let lifecycle =
        match claria_storage::state::load_state::<ClientDeletionState>(s3, bucket, lifecycle_key)
            .await
        {
            Ok((existing, _)) => existing,
            Err(StorageError::NotFound { .. }) => {
                let output = claria_storage::objects::get_object(s3, bucket, client_key)
                    .await
                    .map_err(|source| storage("validating the client", source))?;
                validate_client_body(&output.body, client_id)?;
                let now = jiff::Timestamp::now();
                let created = ClientDeletionState {
                    schema_version: LIFECYCLE_SCHEMA_VERSION,
                    client_id,
                    phase: ClientDeletionPhase::Started,
                    started_at: now,
                    updated_at: now,
                };
                match claria_storage::state::save_state_if_none_match(
                    s3,
                    bucket,
                    lifecycle_key,
                    &created,
                )
                .await
                {
                    Ok(_) => created,
                    Err(StorageError::PreconditionFailed { .. }) => {
                        claria_storage::state::load_state::<ClientDeletionState>(
                            s3,
                            bucket,
                            lifecycle_key,
                        )
                        .await
                        .map(|(state, _)| state)
                        .map_err(|source| storage("loading deletion recovery state", source))?
                    }
                    Err(source) => return Err(storage("starting client deletion", source)),
                }
            }
            Err(source) => return Err(storage("loading deletion recovery state", source)),
        };
    if lifecycle.schema_version != LIFECYCLE_SCHEMA_VERSION || lifecycle.client_id != client_id {
        return Err(ClientLifecycleError::InvalidRecoveryState);
    }
    Ok(lifecycle)
}

fn validate_client_body(body: &[u8], expected_id: Uuid) -> Result<(), ClientLifecycleError> {
    let client: Client =
        serde_json::from_slice(body).map_err(|_| ClientLifecycleError::InvalidClient)?;
    if client.id != expected_id || expected_id.is_nil() {
        return Err(ClientLifecycleError::InvalidClient);
    }
    Ok(())
}

async fn delete_if_exists(
    s3: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<(), ClientLifecycleError> {
    match claria_storage::objects::get_object(s3, bucket, key).await {
        Ok(_) => claria_storage::objects::delete_object(s3, bucket, key)
            .await
            .map_err(|source| storage("clearing lifecycle recovery state", source)),
        Err(StorageError::NotFound { .. }) => Ok(()),
        Err(source) => Err(storage("checking lifecycle recovery state", source)),
    }
}

fn storage(operation: &'static str, source: StorageError) -> ClientLifecycleError {
    ClientLifecycleError::Storage { operation, source }
}

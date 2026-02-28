use std::path::PathBuf;

use aws_sdk_s3::Client as S3Client;

use crate::error::ProvisionerError;
use crate::state::{migrate_state_v1_to_v2, ProvisionerState};

/// Dual-write state persistence: local disk (safety net) + S3 (authoritative).
pub struct StatePersistence {
    pub s3: S3Client,
    pub bucket: String,
    pub s3_key: String,
    pub local_path: PathBuf,
}

impl StatePersistence {
    /// Write state to local disk first (atomic: tmp + rename), then upload to S3.
    ///
    /// Local write happens first so state is never lost even if S3 upload fails.
    pub async fn flush(&self, state: &ProvisionerState) -> Result<(), ProvisionerError> {
        // 1. Atomic local write
        let json = serde_json::to_vec_pretty(state)?;
        if let Some(parent) = self.local_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = self.local_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &self.local_path)?;

        tracing::debug!(path = %self.local_path.display(), "state flushed to local disk");

        // 2. Upload to S3
        match claria_storage::state::save_state(&self.s3, &self.bucket, &self.s3_key, state).await
        {
            Ok(_) => {
                tracing::debug!(
                    bucket = %self.bucket,
                    key = %self.s3_key,
                    "state flushed to S3"
                );
            }
            Err(e) => {
                // Log but don't fail — local write succeeded, S3 is best-effort here.
                // Next load() will pick up the local copy.
                tracing::warn!(
                    error = %e,
                    "failed to upload state to S3 (local copy is safe)"
                );
            }
        }

        Ok(())
    }

    /// Load state: try S3 first (authoritative), fall back to local, return Default if neither.
    ///
    /// When direct deserialization fails (e.g. v1 → v2 schema change), attempts
    /// to load as raw JSON, run migration, and retry. If migration succeeds the
    /// migrated state is flushed back so future loads work directly.
    pub async fn load(&self) -> Result<ProvisionerState, ProvisionerError> {
        // Try S3
        match self.load_from_s3().await {
            Ok(state) => return Ok(state),
            Err(LoadError::NotFound) => {
                tracing::debug!("no state in S3, trying local");
            }
            Err(LoadError::Incompatible(msg)) => {
                return Err(ProvisionerError::State(format!(
                    "Provisioner state is incompatible with this version of Claria: {msg}. \
                     You can reset the provisioner state and re-scan."
                )));
            }
            Err(LoadError::Other(msg)) => {
                tracing::warn!(error = %msg, "failed to load state from S3, trying local");
            }
        }

        // Fall back to local
        match self.load_from_local() {
            Ok(state) => return Ok(state),
            Err(LoadError::NotFound) => {}
            Err(LoadError::Incompatible(msg)) => {
                return Err(ProvisionerError::State(format!(
                    "Local provisioner state is incompatible with this version of Claria: {msg}. \
                     You can reset the provisioner state and re-scan."
                )));
            }
            Err(LoadError::Other(msg)) => {
                tracing::warn!(error = %msg, "failed to load local state");
            }
        }

        // Neither exists — fresh state
        tracing::debug!("no existing state found, starting fresh");
        Ok(ProvisionerState::default())
    }

    /// Delete provisioner state from both local disk and S3.
    pub async fn delete(&self) -> Result<(), ProvisionerError> {
        // Delete local file (ignore not-found).
        match std::fs::remove_file(&self.local_path) {
            Ok(()) => {
                tracing::debug!(path = %self.local_path.display(), "local state deleted");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ProvisionerError::Io(e)),
        }

        // Delete S3 object (ignore not-found).
        match claria_storage::objects::delete_object(&self.s3, &self.bucket, &self.s3_key).await {
            Ok(()) => {
                tracing::debug!(
                    bucket = %self.bucket,
                    key = %self.s3_key,
                    "S3 state deleted"
                );
            }
            Err(claria_storage::error::StorageError::NotFound { .. }) => {}
            Err(e) => return Err(ProvisionerError::Storage(e)),
        }

        Ok(())
    }

    /// Try loading state from S3 with migration fallback.
    async fn load_from_s3(&self) -> Result<ProvisionerState, LoadError> {
        let output = match claria_storage::objects::get_object(
            &self.s3,
            &self.bucket,
            &self.s3_key,
        )
        .await
        {
            Ok(o) => o,
            Err(claria_storage::error::StorageError::NotFound { .. }) => {
                return Err(LoadError::NotFound);
            }
            Err(e) => return Err(LoadError::Other(e.to_string())),
        };

        let bytes = &output.body;

        // Fast path: try direct deserialization.
        match serde_json::from_slice::<ProvisionerState>(bytes) {
            Ok(state) => {
                tracing::debug!(
                    bucket = %self.bucket,
                    key = %self.s3_key,
                    "state loaded from S3"
                );
                return Ok(state);
            }
            Err(direct_err) => {
                tracing::debug!(error = %direct_err, "direct S3 state deserialization failed, trying migration");
            }
        }

        // Slow path: parse as raw JSON, migrate, retry.
        let raw: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| LoadError::Incompatible(e.to_string()))?;
        let migrated = migrate_state_v1_to_v2(raw);
        let state: ProvisionerState = serde_json::from_value(migrated)
            .map_err(|e| LoadError::Incompatible(e.to_string()))?;

        tracing::info!("migrated S3 state from v1 to v2, flushing back");
        if let Err(e) = self.flush(&state).await {
            tracing::warn!(error = %e, "failed to flush migrated state (will retry next load)");
        }

        Ok(state)
    }

    /// Try loading state from local disk with migration fallback.
    fn load_from_local(&self) -> Result<ProvisionerState, LoadError> {
        let bytes = match std::fs::read(&self.local_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(LoadError::NotFound);
            }
            Err(e) => return Err(LoadError::Other(e.to_string())),
        };

        // Fast path: try direct deserialization.
        match serde_json::from_slice::<ProvisionerState>(&bytes) {
            Ok(state) => {
                tracing::debug!(path = %self.local_path.display(), "state loaded from local disk");
                return Ok(state);
            }
            Err(direct_err) => {
                tracing::debug!(error = %direct_err, "direct local state deserialization failed, trying migration");
            }
        }

        // Slow path: parse as raw JSON, migrate, retry.
        let raw: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| LoadError::Incompatible(e.to_string()))?;
        let migrated = migrate_state_v1_to_v2(raw);
        let state: ProvisionerState = serde_json::from_value(migrated)
            .map_err(|e| LoadError::Incompatible(e.to_string()))?;

        tracing::info!(path = %self.local_path.display(), "migrated local state from v1 to v2");
        // Note: we don't flush here — load_from_s3 handles the authoritative write.
        // The local file will be updated on the next flush().

        Ok(state)
    }
}

/// Internal error type for load attempts — distinguishes not-found from
/// incompatible (migration failed) from transient errors.
enum LoadError {
    NotFound,
    Incompatible(String),
    Other(String),
}

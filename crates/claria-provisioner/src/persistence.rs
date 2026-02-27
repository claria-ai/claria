use std::path::PathBuf;

use aws_sdk_s3::Client as S3Client;

use crate::error::ProvisionerError;
use crate::state::ProvisionerState;

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
    pub async fn load(&self) -> Result<ProvisionerState, ProvisionerError> {
        // Try S3
        match claria_storage::state::load_state::<ProvisionerState>(
            &self.s3,
            &self.bucket,
            &self.s3_key,
        )
        .await
        {
            Ok((state, _etag)) => {
                tracing::debug!(bucket = %self.bucket, key = %self.s3_key, "state loaded from S3");
                return Ok(state);
            }
            Err(claria_storage::error::StorageError::NotFound { .. }) => {
                tracing::debug!("no state in S3, trying local");
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load state from S3, trying local");
            }
        }

        // Fall back to local
        if self.local_path.exists() {
            let json = std::fs::read(&self.local_path)?;
            let state: ProvisionerState = serde_json::from_slice(&json)?;
            tracing::debug!(path = %self.local_path.display(), "state loaded from local disk");
            return Ok(state);
        }

        // Neither exists — fresh state
        tracing::debug!("no existing state found, starting fresh");
        Ok(ProvisionerState::default())
    }
}

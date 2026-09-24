use aws_sdk_transcribe::{Client, error::ProvideErrorMetadata};
use serde_json::json;

use crate::{
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
    syncers::read_failed,
};

/// A job name no account will ever have.
///
/// `GetTranscriptionJob` on it answers `BadRequestException` when the caller
/// holds the permission and `AccessDeniedException` when it does not, and AWS
/// bills neither. Transcribe has no describe-service call, so asking about a
/// job that cannot exist is the cheapest honest question available.
const PROBE_JOB_NAME: &str = "claria-access-probe-0000-0000-0000-0000";

pub struct TranscribeAccessSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl TranscribeAccessSyncer {
    pub fn new(spec: ResourceSpec, config: &aws_config::SdkConfig) -> Self {
        Self {
            spec,
            client: Client::new(config),
        }
    }
}

impl ResourceSyncer for TranscribeAccessSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_transcription_job()
                .transcription_job_name(PROBE_JOB_NAME)
                .send()
                .await
            {
                // A job by that name really would have to exist for this to
                // happen. Either way the call was allowed.
                Ok(_) => Ok(Some(json!({"access": "granted"}))),
                Err(e) => {
                    // The service answered, which is the whole question. It
                    // reports a name it cannot find as `BadRequest` rather
                    // than `NotFound`, so both count.
                    let answered = matches!(
                        e.code(),
                        Some("BadRequestException") | Some("NotFoundException")
                    );
                    if answered {
                        return Ok(Some(json!({"access": "granted"})));
                    }
                    Err(read_failed("transcribe:GetTranscriptionJob", &e))
                }
            }
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}

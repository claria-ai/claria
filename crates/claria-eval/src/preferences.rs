//! Which models a run uses, resolved the way the desktop resolves them.
//!
//! The planner and writer IDs come from the clinician's synced preferences in
//! S3 (`_state/preferences.json`), not from the local config, because that is
//! the copy the desktop app treats as authoritative. Only the two fields this
//! harness needs are declared; everything else in the object is ignored.

use eyre::{Context, Result};
use serde::Deserialize;

use crate::EvalContext;

/// The subset of the desktop's `SyncedPreferences` this harness reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvalPreferences {
    #[serde(default)]
    pub preferred_model_id: Option<String>,
    #[serde(default)]
    pub draft_pipeline: EvalDraftPipeline,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvalDraftPipeline {
    #[serde(default)]
    pub planner_model_id: Option<String>,
}

/// The two models one pass has to fit inside.
#[derive(Debug, Clone)]
pub struct ResolvedModels {
    pub planner_model_id: String,
    pub writer_model_id: String,
}

/// Read `_state/preferences.json`. A missing object is first launch, not an
/// error.
pub async fn load(context: &EvalContext) -> Result<EvalPreferences> {
    match claria_storage::objects::get_object(
        &context.s3,
        &context.bucket,
        claria_core::s3_keys::PREFERENCES,
    )
    .await
    {
        Ok(output) => serde_json::from_slice(&output.body)
            .wrap_err("the synced preferences object did not parse"),
        Err(claria_storage::error::StorageError::NotFound { .. }) => Ok(EvalPreferences::default()),
        Err(error) => Err(error).wrap_err("could not read the synced preferences"),
    }
}

/// Resolve the planner and writer models for a pass.
///
/// Mirrors `claria_desktop::commands::plan::role_model_id`: an override is
/// honoured only when the account can actually reach it, and otherwise the
/// capability table picks the default. Supplying both overrides on the
/// command line skips model discovery entirely.
pub async fn resolve(
    context: &EvalContext,
    planner_override: Option<&str>,
    writer_override: Option<&str>,
) -> Result<ResolvedModels> {
    if let (Some(planner), Some(writer)) = (planner_override, writer_override) {
        return Ok(ResolvedModels {
            planner_model_id: planner.to_string(),
            writer_model_id: writer.to_string(),
        });
    }

    let preferences = load(context).await?;
    let writer_model_id = writer_override
        .map(str::to_string)
        .or(preferences.preferred_model_id)
        .unwrap_or_default();
    let planner_choice = planner_override
        .map(str::to_string)
        .or(preferences.draft_pipeline.planner_model_id);

    let discovered: Vec<String> = claria_bedrock::chat::list_chat_models(&context.sdk_config)
        .await
        .wrap_err("could not list the models this account can reach")?
        .into_iter()
        .map(|model| model.model_id)
        .collect();
    let planner_model_id = claria_core::model_id::resolve_role_model(
        planner_choice.as_deref(),
        &discovered,
        &writer_model_id,
    )
    .to_string();

    // Plan time does not fix the writing model — the gate's Start button
    // does — but the record corpus has to fit its window, so a run with no
    // saved preference sizes against the planner.
    let writer_model_id = if writer_model_id.is_empty() {
        planner_model_id.clone()
    } else {
        writer_model_id
    };

    Ok(ResolvedModels {
        planner_model_id,
        writer_model_id,
    })
}

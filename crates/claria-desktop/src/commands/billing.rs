//! Cost Explorer commands and the Bedrock pricing lookup.

use tauri::State;

use super::{
    CommandContext,
    config::{PreferencesPatch, apply_preferences_patch},
    run,
};
use crate::state::DesktopState;

#[tauri::command]
#[specta::specta]
pub async fn get_cost_and_usage(
    state: State<'_, DesktopState>,
    start_date: String,
    end_date: String,
    granularity: claria_billing::CostGranularity,
    group_by_service: bool,
) -> Result<claria_billing::CostAndUsageResult, String> {
    run("get_cost_and_usage", async {
        let ctx = CommandContext::new(&state).await?;
        let query = claria_billing::CostQuery {
            start_date,
            end_date,
            granularity,
            group_by_service,
        };
        Ok(claria_billing::get_cost_and_usage(&ctx.sdk_config, &query).await?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn probe_cost_explorer(state: State<'_, DesktopState>) -> Result<(), String> {
    run("probe_cost_explorer", async {
        let ctx = CommandContext::new(&state).await?;
        Ok(claria_billing::probe_cost_explorer(&ctx.sdk_config).await?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn enable_cost_explorer(state: State<'_, DesktopState>) -> Result<(), String> {
    run("enable_cost_explorer", async {
        let ctx = CommandContext::new(&state).await?;
        let patch = PreferencesPatch {
            cost_explorer_enabled: Some(true),
            ..Default::default()
        };
        apply_preferences_patch(&state, &ctx.s3, ctx.cfg, &patch, "Cost Explorer setting").await?;
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_hourly_cost_data(
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<(), String> {
    run("set_hourly_cost_data", async {
        let ctx = CommandContext::new(&state).await?;
        let patch = PreferencesPatch {
            hourly_cost_data: Some(enabled),
            ..Default::default()
        };
        apply_preferences_patch(&state, &ctx.s3, ctx.cfg, &patch, "hourly cost data setting")
            .await?;
        Ok(())
    })
    .await
}

/// Look up `ModelPricing` for a Bedrock model_id. Returns `None` for
/// unknown models so the UI can hide pre-flight estimates rather than
/// show `$NaN`.
#[tauri::command]
#[specta::specta]
pub async fn lookup_model_pricing(
    model_id: String,
) -> Result<Option<claria_core::models::cost::ModelPricing>, String> {
    Ok(claria_billing::pricing::lookup(&model_id))
}

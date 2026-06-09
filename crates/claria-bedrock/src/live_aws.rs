//! `LiveAwsValueProvider` impls for Bedrock-side catalog values.
//!
//! - [`BedrockChatModelProvider`] — UserChoice: the user's preferred chat
//!   model. Discovery filters inference profiles to the user's geography.
//! - [`BedrockMarketplaceAgreementProvider`] — ProvisionerContributor:
//!   contributes one `ResourceSpec` per active Anthropic foundation model so
//!   the provisioner's existing syncer accepts agreements on the user's
//!   behalf under elevation.

use aws_config::SdkConfig;
use aws_sdk_bedrock::types::{
    FoundationModelLifecycleStatus, InferenceProfileStatus, InferenceProfileType,
};
use serde_json::{json, Value};

use claria_live_aws::error::ProviderError;
use claria_live_aws::provider::{
    BoxFuture, Candidate, ChangeReason, LiveAwsValueProvider, ProviderKind, Reconciliation,
};
use claria_provisioner::{
    CredentialScope, Lifecycle, ManifestContributor, ProvisionerError, ResourceSpec, Severity,
};

use crate::geography::{
    parse_inference_profile_id, AvailableProfile, FoundationModelStatus, KNOWN_GEOGRAPHIES,
};

/// Tauri command callers supply the user's current geography preference so
/// the provider can scope its discovery.
pub struct BedrockChatModelProvider {
    /// User's geography preference. `"us"`/`"eu"`/`"global"` etc.
    geography: String,
    /// `"bedrock.chat_model"` or `"bedrock.extraction_model"` — same logic,
    /// different config slot.
    key: &'static str,
    /// Recommend the cheapest Sonnet (true) or newest Sonnet (false).
    /// Extraction prefers cheapest; chat prefers newest.
    prefer_cheapest: bool,
}

impl BedrockChatModelProvider {
    pub fn chat(geography: impl Into<String>) -> Self {
        Self {
            geography: geography.into(),
            key: "bedrock.chat_model",
            prefer_cheapest: false,
        }
    }

    pub fn extraction(geography: impl Into<String>) -> Self {
        Self {
            geography: geography.into(),
            key: "bedrock.extraction_model",
            prefer_cheapest: true,
        }
    }
}

impl LiveAwsValueProvider for BedrockChatModelProvider {
    fn key(&self) -> &'static str {
        self.key
    }

    fn label(&self) -> &'static str {
        if self.key == "bedrock.chat_model" {
            "Bedrock chat model"
        } else {
            "Bedrock extraction model"
        }
    }

    fn description(&self) -> &'static str {
        "The Bedrock inference profile used for AI chat. Sourced from \
         AWS's current catalog — selecting a new model never requires a Claria update."
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::UserChoice
    }

    fn reconcile<'a>(
        &'a self,
        sdk: &'a SdkConfig,
        current: &'a Value,
    ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
        Box::pin(async move {
            // Validate geography first — defensive
            if !KNOWN_GEOGRAPHIES.contains(&self.geography.as_str()) {
                return Err(ProviderError::AwsApi(format!(
                    "unknown geography preference {:?}",
                    self.geography
                )));
            }

            let profiles = list_profiles_in_geography(sdk, &self.geography).await?;

            // Classify current
            let validity = classify_current(current, &profiles, &self.geography);

            match validity {
                CurrentValidity::Active => Ok(Reconciliation::InSync),
                CurrentValidity::NeverConfigured => {
                    let candidates = build_candidates(&profiles, self.prefer_cheapest);
                    Ok(Reconciliation::UserChoiceDrift {
                        reason: ChangeReason::NeverConfigured,
                        candidates,
                        current: None,
                        summary: format!(
                            "Pick a {} chat model",
                            self.geography.to_uppercase()
                        ),
                    })
                }
                CurrentValidity::Retired => {
                    let candidates = build_candidates(&profiles, self.prefer_cheapest);
                    Ok(Reconciliation::UserChoiceDrift {
                        reason: ChangeReason::Retired,
                        candidates,
                        current: Some(current.clone()),
                        summary: "Your configured model is no longer available — pick a replacement".into(),
                    })
                }
                CurrentValidity::Deprecation { eol } => {
                    let candidates = build_candidates(&profiles, self.prefer_cheapest);
                    Ok(Reconciliation::UserChoiceDrift {
                        reason: ChangeReason::Deprecation { eol },
                        candidates,
                        current: Some(current.clone()),
                        summary: "Your configured model is approaching end-of-life — consider switching".into(),
                    })
                }
                CurrentValidity::PreferenceMismatch => {
                    let candidates = build_candidates(&profiles, self.prefer_cheapest);
                    Ok(Reconciliation::UserChoiceDrift {
                        reason: ChangeReason::PreferenceMismatch,
                        candidates,
                        current: Some(current.clone()),
                        summary: format!(
                            "Your model is in a different geography — pick one in {}",
                            self.geography
                        ),
                    })
                }
                CurrentValidity::Malformed => {
                    let candidates = build_candidates(&profiles, self.prefer_cheapest);
                    Ok(Reconciliation::UserChoiceDrift {
                        reason: ChangeReason::Malformed,
                        candidates,
                        current: Some(current.clone()),
                        summary: "Your stored model ID is unrecognizable — pick again".into(),
                    })
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub enum CurrentValidity {
    Active,
    NeverConfigured,
    Retired,
    Deprecation { eol: String },
    PreferenceMismatch,
    Malformed,
}

/// Decide what state the user's currently-stored value is in, given the live
/// AWS profile list for the user's geography.
///
/// Pure inspection — no AWS calls. Visible for unit testing.
pub fn classify_current(
    current: &Value,
    profiles_in_geo: &[AvailableProfile],
    expected_geo: &str,
) -> CurrentValidity {
    let Some(s) = current.as_str() else {
        return CurrentValidity::NeverConfigured;
    };
    if s.is_empty() {
        return CurrentValidity::NeverConfigured;
    }
    if s.starts_with("arn:") {
        return CurrentValidity::Malformed;
    }
    let Some((geo, _)) = parse_inference_profile_id(s) else {
        // 2-segment bare model ID or pure garbage
        return CurrentValidity::Malformed;
    };
    if geo != expected_geo {
        return CurrentValidity::PreferenceMismatch;
    }
    // Look up in the geo's active set
    match profiles_in_geo.iter().find(|p| p.profile_id == s) {
        Some(p) => match p.underlying_status {
            FoundationModelStatus::Active => CurrentValidity::Active,
            FoundationModelStatus::Legacy { eol } => CurrentValidity::Deprecation {
                eol: eol.to_string(),
            },
            FoundationModelStatus::Eol => CurrentValidity::Retired,
        },
        None => CurrentValidity::Retired,
    }
}

/// Build the candidate list for the user's geography. Filters to ACTIVE
/// underlying models only (LEGACY/EOL appear in `classify_current` as
/// `Deprecation`/`Retired` but aren't offered as new selections).
pub fn build_candidates(
    profiles_in_geo: &[AvailableProfile],
    prefer_cheapest: bool,
) -> Vec<Candidate> {
    let mut active: Vec<&AvailableProfile> = profiles_in_geo
        .iter()
        .filter(|p| matches!(p.underlying_status, FoundationModelStatus::Active))
        .collect();
    active.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));

    if active.is_empty() {
        return vec![];
    }

    let recommended_id = pick_recommendation(&active, prefer_cheapest);

    active
        .iter()
        .map(|p| Candidate {
            value: json!(p.profile_id.clone()),
            label: humanize_profile(&p.profile_id),
            description: describe_profile(&p.profile_id),
            recommended: Some(&p.profile_id) == recommended_id.as_ref(),
            metadata: json!({
                "profile_id": p.profile_id,
                "scope": parse_inference_profile_id(&p.profile_id).map(|(g, _)| g).unwrap_or(""),
            }),
        })
        .collect()
}

fn pick_recommendation(
    candidates: &[&AvailableProfile],
    prefer_cheapest: bool,
) -> Option<String> {
    // Prefer Sonnet over Opus (cost-conscious default per feedback_cost_conscious_defaults).
    // For "chat" (newest), pick the highest-sorting Sonnet ID. For "extraction"
    // (cheapest), pick the lowest-sorting Sonnet ID. Both fall back to whatever
    // is available if no Sonnet.
    let sonnets: Vec<&&AvailableProfile> = candidates
        .iter()
        .filter(|p| p.profile_id.contains("sonnet"))
        .collect();
    let pool = if !sonnets.is_empty() {
        &sonnets
    } else {
        &candidates.iter().collect::<Vec<_>>()
    };
    if pool.is_empty() {
        return None;
    }
    let pick = if prefer_cheapest {
        pool.iter().min_by(|a, b| a.profile_id.cmp(&b.profile_id))?
    } else {
        pool.iter().max_by(|a, b| a.profile_id.cmp(&b.profile_id))?
    };
    Some(pick.profile_id.clone())
}

fn humanize_profile(id: &str) -> String {
    // "us.anthropic.claude-sonnet-4-20250514-v1:0" → "Claude Sonnet 4 (us)"
    let Some((geo, rest)) = parse_inference_profile_id(id) else {
        return id.into();
    };
    let name = rest.trim_start_matches("anthropic.");
    let pretty = name
        .split('-')
        .take(3)
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{pretty} ({geo})")
}

fn describe_profile(id: &str) -> String {
    if id.contains("sonnet") {
        "Mid-tier model — recommended for most chat and extraction tasks".into()
    } else if id.contains("opus") {
        "High-tier model — slower and more expensive than Sonnet".into()
    } else if id.contains("haiku") {
        "Lightweight model — fastest, cheapest".into()
    } else {
        format!("Anthropic model {id}")
    }
}

/// Query `ListInferenceProfiles` + `ListFoundationModels` and return the
/// merged catalog filtered to the requested geography.
pub async fn list_profiles_in_geography(
    sdk: &SdkConfig,
    geography: &str,
) -> Result<Vec<AvailableProfile>, ProviderError> {
    let client = aws_sdk_bedrock::Client::new(sdk);
    let geo_prefix = format!("{geography}.");

    // Build a foundation-model-id -> lifecycle status map first
    let fm_response = client
        .list_foundation_models()
        .by_provider("anthropic")
        .send()
        .await
        .map_err(|e| ProviderError::AwsApi(e.into_service_error().to_string()))?;

    let mut fm_status: std::collections::HashMap<String, FoundationModelStatus> =
        std::collections::HashMap::new();
    for m in fm_response.model_summaries() {
        let id = m.model_id();
        let status = match m.model_lifecycle() {
            Some(lc) => match lc.status() {
                FoundationModelLifecycleStatus::Active => FoundationModelStatus::Active,
                FoundationModelLifecycleStatus::Legacy => {
                    // Best-effort: use now + 90d if no end_of_life_time is exposed
                    let eol = jiff::Timestamp::now()
                        .checked_add(jiff::SignedDuration::from_secs(60 * 60 * 24 * 90))
                        .unwrap_or_else(|_| jiff::Timestamp::now());
                    FoundationModelStatus::Legacy { eol }
                }
                _ => FoundationModelStatus::Eol,
            },
            None => FoundationModelStatus::Active,
        };
        fm_status.insert(id.to_string(), status);
    }

    // Then list inference profiles and join
    let ip_response = client
        .list_inference_profiles()
        .type_equals(InferenceProfileType::SystemDefined)
        .max_results(100)
        .send()
        .await
        .map_err(|e| ProviderError::AwsApi(e.into_service_error().to_string()))?;

    let mut profiles = vec![];
    for p in ip_response.inference_profile_summaries() {
        let id = p.inference_profile_id();
        if !id.starts_with(&geo_prefix) {
            continue;
        }
        if !id.contains("anthropic.claude") {
            continue;
        }
        if *p.status() != InferenceProfileStatus::Active {
            continue;
        }
        // Resolve underlying status by stripping the geo prefix
        let bare = &id[geo_prefix.len()..];
        let underlying_status = fm_status
            .get(bare)
            .copied()
            .unwrap_or(FoundationModelStatus::Active);
        profiles.push(AvailableProfile {
            profile_id: id.to_string(),
            underlying_status,
        });
    }

    Ok(profiles)
}

// ── Marketplace agreement provider ───────────────────────────────────────────

/// Discovers active Anthropic foundation models at runtime and contributes
/// one `ResourceSpec` per model into the provisioner manifest.
///
/// Implements both `LiveAwsValueProvider` (so it shows up in the framework's
/// reconcile pass) and `ManifestContributor` (so the provisioner build
/// pipeline can include its specs directly when called outside the framework
/// flow, e.g. from `provision_scan`).
pub struct BedrockMarketplaceAgreementProvider;

impl BedrockMarketplaceAgreementProvider {
    pub fn new() -> Self {
        Self
    }

    async fn discover_active_models(
        &self,
        sdk: &SdkConfig,
    ) -> Result<Vec<String>, ProviderError> {
        let client = aws_sdk_bedrock::Client::new(sdk);
        let response = client
            .list_foundation_models()
            .by_provider("anthropic")
            .send()
            .await
            .map_err(|e| ProviderError::AwsApi(e.into_service_error().to_string()))?;

        let models: Vec<String> = response
            .model_summaries()
            .iter()
            .filter(|m| {
                let is_active = m
                    .model_lifecycle()
                    .map(|lc| {
                        *lc.status() == FoundationModelLifecycleStatus::Active
                    })
                    .unwrap_or(false);
                // Skip context-window variants like `:48k`, `:200k`
                let id = m.model_id();
                let is_variant = id.rsplit_once(':').is_some_and(|(_, suffix)| {
                    suffix.chars().next().is_some_and(|c| c.is_ascii_digit())
                        && suffix != "0"
                });
                is_active && !is_variant && id.contains("claude")
            })
            .map(|m| m.model_id().to_string())
            .collect();

        Ok(models)
    }
}

impl Default for BedrockMarketplaceAgreementProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveAwsValueProvider for BedrockMarketplaceAgreementProvider {
    fn key(&self) -> &'static str {
        "bedrock.marketplace_agreements"
    }

    fn label(&self) -> &'static str {
        "Anthropic marketplace agreements"
    }

    fn description(&self) -> &'static str {
        "Accepted marketplace agreements for currently-active Anthropic \
         foundation models. The list is built from AWS at runtime — \
         new models become accessible without a Claria update."
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::ProvisionerContributor
    }

    fn reconcile<'a>(
        &'a self,
        sdk: &'a SdkConfig,
        _current: &'a Value,
    ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>> {
        Box::pin(async move {
            let models = self.discover_active_models(sdk).await?;
            if models.is_empty() {
                return Ok(Reconciliation::InSync);
            }
            let specs = models
                .iter()
                .map(|m| build_marketplace_spec(m))
                .collect::<Vec<_>>();
            let count = specs.len();
            let summary = if count == 1 {
                format!("1 Anthropic marketplace agreement: {}", models[0])
            } else {
                format!("{count} Anthropic marketplace agreements")
            };
            Ok(Reconciliation::Contribution {
                reason: ChangeReason::NewUpstreamResource {
                    name: "Anthropic models".into(),
                },
                specs,
                summary,
            })
        })
    }
}

impl ManifestContributor for BedrockMarketplaceAgreementProvider {
    fn contribute<'a>(
        &'a self,
        sdk: &'a SdkConfig,
    ) -> claria_provisioner::syncer::BoxFuture<
        'a,
        Result<Vec<ResourceSpec>, ProvisionerError>,
    > {
        Box::pin(async move {
            let models = self.discover_active_models(sdk).await.map_err(|e| {
                ProvisionerError::State(format!("failed to discover Anthropic models: {e}"))
            })?;
            Ok(models.iter().map(|m| build_marketplace_spec(m)).collect())
        })
    }
}

fn build_marketplace_spec(model_id: &str) -> ResourceSpec {
    let pretty = model_id.trim_start_matches("anthropic.");
    let label_pretty = pretty
        .split('-')
        .take(3)
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    ResourceSpec {
        resource_type: "bedrock_model_agreement".into(),
        resource_name: model_id.into(),
        lifecycle: Lifecycle::Managed,
        desired: json!({"agreement": "accepted"}),
        credential_scope: CredentialScope::Regular,
        label: format!("{label_pretty} Access"),
        description: format!("AI model access for {label_pretty}"),
        severity: Severity::Elevated,
        iam_actions: vec![
            "bedrock:ListFoundationModels".into(),
            "bedrock:ListInferenceProfiles".into(),
            "bedrock:GetFoundationModelAvailability".into(),
            "bedrock:ListFoundationModelAgreementOffers".into(),
            "bedrock:CreateFoundationModelAgreement".into(),
            "bedrock:InvokeModel".into(),
            "bedrock:InvokeModelWithResponseStream".into(),
            "bedrock:CountTokens".into(),
            "aws-marketplace:ViewSubscriptions".into(),
            "aws-marketplace:Subscribe".into(),
        ],
    }
}

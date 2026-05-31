//! Bedrock model-access enrollment: enumerate Anthropic Claude model
//! agreements, surface their terms, and let the user explicitly execute (sign
//! up for) them.
//!
//! This is the user-driven replacement for the old silent auto-accept. Model
//! discovery helpers (active foundation models, US inference profiles, scope
//! stripping) live in [`crate::chat`] and are reused here.
//!
//! ## How Bedrock gates Anthropic Claude
//!
//! Access to a Claude model on Bedrock requires an AWS Marketplace agreement,
//! created via `CreateFoundationModelAgreement`. Two things make this subtle:
//!
//! 1. **Anthropic first-time-use (FTU) form.** Anthropic gates *all* its
//!    Bedrock models behind a one-time, per-account "use case" form
//!    (`PutUseCaseForModelAccess`). Until it's submitted no agreement can be
//!    executed and no model invoked. [`get_use_case_form`] reads its status;
//!    [`submit_use_case_form`] submits it.
//! 2. **Agreement creation is asynchronous.** `CreateFoundationModelAgreement`
//!    returns immediately (HTTP 202); the marketplace subscription provisions
//!    in the background and can take minutes (status `PENDING`). We therefore
//!    do *not* re-read availability and declare failure — [`execute_agreement`]
//!    returns once the request is accepted and the caller polls
//!    [`get_enrollment`] until the model becomes [`EnrollmentStatus::Executed`].
//!
//! The authoritative "can invoke" signal is
//! `entitlement_availability == AVAILABLE`; a successful Converse call is the
//! ultimate confirmation (control-plane status can read ready slightly early).

use aws_sdk_bedrock::types::OfferType;
use aws_sdk_bedrock::operation::get_foundation_model_availability::GetFoundationModelAvailabilityOutput;
use aws_smithy_types::Blob;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::chat;
use crate::error::{BedrockError, ModelAccessReason};

// ── Types ────────────────────────────────────────────────────────────────────

/// Per-model enrollment status, computed from the four availability axes
/// Bedrock returns (region / entitlement / agreement / authorization).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EnrollmentStatus {
    /// Entitled — the model is invokable right now.
    Executed,
    /// A marketplace agreement is available to sign up for (and the FTU form is
    /// already submitted).
    Available,
    /// An agreement was requested; the marketplace subscription is still
    /// provisioning. Poll until it flips to [`EnrollmentStatus::Executed`].
    Pending,
    /// The Anthropic use-case form must be submitted before this model can be
    /// enrolled.
    UseCaseFormRequired,
    /// The model isn't offered in the configured region.
    RegionUnavailable,
    /// Not actionable from here (agreement errored, not authorized, or no
    /// marketplace flow exists). The reason is shown to the user verbatim.
    Blocked { reason: String },
}

/// One Anthropic Claude model the user can enroll in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEnrollment {
    /// Bare foundation model id — the agreement/availability key, e.g.
    /// `anthropic.claude-opus-4-...`.
    pub model_id: String,
    /// Inference profile id used to actually invoke the model, e.g.
    /// `us.anthropic.claude-opus-4-...`.
    pub inference_profile_id: String,
    /// Human-readable model name.
    pub name: String,
    pub status: EnrollmentStatus,
    /// Offer terms to show before sign-up. `None` when no marketplace offer
    /// exists or the terms couldn't be fetched.
    pub offer: Option<OfferTerms>,
}

/// Offer terms surfaced to the user before they execute an agreement — a subset
/// of the AWS `Offer` / `TermDetails`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferTerms {
    pub offer_token: String,
    pub offer_id: Option<String>,
    /// EULA / legal terms URL (`legal_term.url`), opened in the browser.
    pub legal_terms_url: Option<String>,
    pub refund_policy: Option<String>,
    pub agreement_duration: Option<String>,
    pub pricing: Vec<PricingRate>,
}

/// One row of an offer's usage-based pricing rate card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingRate {
    pub dimension: Option<String>,
    pub description: Option<String>,
    pub price: Option<String>,
    pub unit: Option<String>,
}

/// The Anthropic first-time-use form. Submitted once per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseCaseForm {
    pub company_name: String,
    pub company_website: String,
    /// 0 = internal employees, 1 = third parties, 2 = both (AWS FTU enum).
    pub intended_users: u8,
    pub industry_option: String,
    pub other_industry_option: Option<String>,
    pub use_cases: String,
}

/// Whether the account-level FTU form has been submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseCaseFormStatus {
    pub submitted: bool,
    /// The previously-submitted form, when it could be read back.
    pub form: Option<UseCaseForm>,
}

/// Wire form for the FTU `formData` blob. AWS expects camelCase keys, whereas
/// the rest of the codebase serializes snake_case — so the blob gets its own
/// representation and we convert at the boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireUseCaseForm {
    company_name: String,
    company_website: String,
    intended_users: u8,
    industry_option: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    other_industry_option: Option<String>,
    use_cases: String,
}

impl From<&UseCaseForm> for WireUseCaseForm {
    fn from(f: &UseCaseForm) -> Self {
        Self {
            company_name: f.company_name.clone(),
            company_website: f.company_website.clone(),
            intended_users: f.intended_users,
            industry_option: f.industry_option.clone(),
            other_industry_option: f.other_industry_option.clone(),
            use_cases: f.use_cases.clone(),
        }
    }
}

impl From<WireUseCaseForm> for UseCaseForm {
    fn from(w: WireUseCaseForm) -> Self {
        Self {
            company_name: w.company_name,
            company_website: w.company_website,
            intended_users: w.intended_users,
            industry_option: w.industry_option,
            other_industry_option: w.other_industry_option,
            use_cases: w.use_cases,
        }
    }
}

// ── Classification ─────────────────────────────────────────────────────────

/// Classify the four availability axes (as their wire strings) into a status.
///
/// Priority order matters: a region miss or an existing entitlement short-circuit
/// before we look at the agreement state. Kept string-typed (not AWS enums) so
/// it's trivially unit-testable.
pub fn classify_axes(
    region_availability: &str,
    entitlement_availability: &str,
    agreement_status: Option<&str>,
    agreement_error: Option<&str>,
    authorization_status: &str,
) -> EnrollmentStatus {
    if region_availability != "AVAILABLE" {
        return EnrollmentStatus::RegionUnavailable;
    }
    if entitlement_availability == "AVAILABLE" {
        return EnrollmentStatus::Executed;
    }
    match agreement_status {
        Some("AVAILABLE") => EnrollmentStatus::Available,
        Some("PENDING") => EnrollmentStatus::Pending,
        Some("ERROR") => EnrollmentStatus::Blocked {
            reason: agreement_error
                .unwrap_or("the marketplace agreement is in an error state")
                .to_string(),
        },
        // NOT_AVAILABLE or no agreement record: nothing we can sign up for here.
        _ => {
            if authorization_status == "AUTHORIZED" {
                EnrollmentStatus::Blocked {
                    reason: "no marketplace agreement is available for this model".to_string(),
                }
            } else {
                EnrollmentStatus::Blocked {
                    reason: "this account isn't authorized for this model".to_string(),
                }
            }
        }
    }
}

/// Serialize the FTU form to the camelCase JSON bytes AWS's `formData` expects.
pub fn use_case_form_to_bytes(form: &UseCaseForm) -> Result<Vec<u8>, BedrockError> {
    Ok(serde_json::to_vec(&WireUseCaseForm::from(form))?)
}

/// Parse the FTU form back from AWS's camelCase JSON `formData` bytes, returning
/// `None` if the payload doesn't match the expected shape.
pub fn use_case_form_from_bytes(bytes: &[u8]) -> Option<UseCaseForm> {
    serde_json::from_slice::<WireUseCaseForm>(bytes)
        .ok()
        .map(UseCaseForm::from)
}

fn classify(avail: &GetFoundationModelAvailabilityOutput) -> EnrollmentStatus {
    classify_axes(
        avail.region_availability().as_str(),
        avail.entitlement_availability().as_str(),
        avail.agreement_availability().map(|a| a.status().as_str()),
        avail.agreement_availability().and_then(|a| a.error_message()),
        avail.authorization_status().as_str(),
    )
}

// ── Public API ───────────────────────────────────────────────────────────────

/// List enrollment state for every active Anthropic Claude model Claria uses.
///
/// Per-model availability failures fold into [`EnrollmentStatus::Blocked`] so a
/// single bad model never aborts the list. Results are sorted by name.
pub async fn list_enrollments(
    config: &aws_config::SdkConfig,
) -> Result<Vec<ModelEnrollment>, BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    let active = chat::fetch_active_foundation_models(&client).await?;
    let us_profiles = chat::fetch_us_inference_profiles(&client).await?;

    // FTU gating is account-wide: read it once. A genuine error (e.g. denied)
    // bubbles so the page can tell the user to refresh their IAM policy.
    let ftu_submitted = get_use_case_form(config).await?.submitted;

    let mut enrollments = Vec::with_capacity(active.len());
    for (model_id, model_name) in active {
        let mut status = match client
            .get_foundation_model_availability()
            .model_id(&model_id)
            .send()
            .await
        {
            Ok(avail) => classify(&avail),
            Err(e) => {
                let msg = e.into_service_error().to_string();
                tracing::warn!(model_id, error = %msg, "GetFoundationModelAvailability failed");
                EnrollmentStatus::Blocked {
                    reason: format!("availability check failed: {msg}"),
                }
            }
        };

        // Until the FTU form is in, an otherwise-available model can't be
        // enrolled — surface the form requirement instead.
        if !ftu_submitted && status == EnrollmentStatus::Available {
            status = EnrollmentStatus::UseCaseFormRequired;
        }

        let offer = match status {
            EnrollmentStatus::Available
            | EnrollmentStatus::Pending
            | EnrollmentStatus::UseCaseFormRequired => fetch_offer(&client, &model_id).await,
            _ => None,
        };

        let inference_profile_id = us_profiles
            .get(&model_id)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| format!("us.{model_id}"));

        enrollments.push(ModelEnrollment {
            model_id,
            inference_profile_id,
            name: model_name,
            status,
            offer,
        });
    }

    enrollments.sort_by(|a, b| a.name.cmp(&b.name));
    info!(count = enrollments.len(), "listed model enrollments");
    Ok(enrollments)
}

/// Fetch one model's current enrollment — the poll target after an execute.
pub async fn get_enrollment(
    config: &aws_config::SdkConfig,
    model_id: &str,
) -> Result<ModelEnrollment, BedrockError> {
    list_enrollments(config)
        .await?
        .into_iter()
        .find(|m| m.model_id == model_id)
        .ok_or_else(|| {
            BedrockError::Agreement(format!("model {model_id} is not an enrollable Claude model"))
        })
}

/// Read the account's first-time-use form status.
///
/// A "form not submitted" response maps to `submitted: false` (not an error);
/// any other failure bubbles.
pub async fn get_use_case_form(
    config: &aws_config::SdkConfig,
) -> Result<UseCaseFormStatus, BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    match client.get_use_case_for_model_access().send().await {
        Ok(resp) => Ok(UseCaseFormStatus {
            submitted: true,
            form: use_case_form_from_bytes(resp.form_data().as_ref()),
        }),
        Err(e) => {
            let msg = e.into_service_error().to_string();
            if is_ftu_not_filled(&msg) {
                Ok(UseCaseFormStatus {
                    submitted: false,
                    form: None,
                })
            } else {
                Err(BedrockError::Agreement(msg))
            }
        }
    }
}

/// Submit the account's first-time-use form (once per account).
pub async fn submit_use_case_form(
    config: &aws_config::SdkConfig,
    form: &UseCaseForm,
) -> Result<(), BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    client
        .put_use_case_for_model_access()
        .form_data(Blob::new(use_case_form_to_bytes(form)?))
        .send()
        .await
        .map_err(|e| BedrockError::Agreement(e.into_service_error().to_string()))?;

    info!("submitted Bedrock use-case form");
    Ok(())
}

/// Execute (sign up for) one model's marketplace agreement.
///
/// Accepts the model's first available offer. `CreateFoundationModelAgreement`
/// is asynchronous — this returns once the request is accepted; the caller polls
/// [`get_enrollment`] for the transition to [`EnrollmentStatus::Executed`].
pub async fn execute_agreement(
    config: &aws_config::SdkConfig,
    model_id: &str,
) -> Result<(), BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    let offers = client
        .list_foundation_model_agreement_offers()
        .model_id(model_id)
        .offer_type(OfferType::All)
        .send()
        .await
        .map_err(|e| BedrockError::Agreement(e.into_service_error().to_string()))?;

    let Some(first) = offers.offers().first() else {
        return Err(BedrockError::Agreement(format!(
            "no marketplace offer is available for {model_id}"
        )));
    };
    let offer_token = first.offer_token().to_string();

    info!(model_id, "executing model agreement");

    match client
        .create_foundation_model_agreement()
        .model_id(model_id)
        .offer_token(&offer_token)
        .send()
        .await
    {
        Ok(_) => {
            info!(model_id, "model agreement requested");
            Ok(())
        }
        Err(e) => {
            let msg = e.into_service_error().to_string();
            if msg.contains("already exists") {
                info!(model_id, "model agreement already exists");
                Ok(())
            } else if is_ftu_not_filled(&msg) {
                Err(BedrockError::ModelAccess {
                    model_id: model_id.to_string(),
                    reason: ModelAccessReason::UseCaseFormRequired,
                })
            } else {
                Err(BedrockError::Agreement(msg))
            }
        }
    }
}

/// Un-enroll a model by deleting its agreement. Note: re-invoking the model
/// re-creates the agreement, so this is informational rather than a hard block.
pub async fn delete_agreement(
    config: &aws_config::SdkConfig,
    model_id: &str,
) -> Result<(), BedrockError> {
    let client = aws_sdk_bedrock::Client::new(config);

    client
        .delete_foundation_model_agreement()
        .model_id(model_id)
        .send()
        .await
        .map_err(|e| BedrockError::Agreement(e.into_service_error().to_string()))?;

    info!(model_id, "deleted model agreement");
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Fetch the first offer's terms for a model. Offers are display-only, so a
/// failure degrades to `None` (logged) rather than failing the whole list.
async fn fetch_offer(client: &aws_sdk_bedrock::Client, model_id: &str) -> Option<OfferTerms> {
    let offers = match client
        .list_foundation_model_agreement_offers()
        .model_id(model_id)
        .offer_type(OfferType::All)
        .send()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            let msg = e.into_service_error().to_string();
            tracing::warn!(model_id, error = %msg, "ListFoundationModelAgreementOffers failed");
            return None;
        }
    };

    let first = offers.offers().first()?;
    let terms = first.term_details();

    let pricing = terms
        .and_then(|t| t.usage_based_pricing_term())
        .map(|p| {
            p.rate_card()
                .iter()
                .map(|r| PricingRate {
                    dimension: r.dimension().map(str::to_string),
                    description: r.description().map(str::to_string),
                    price: r.price().map(str::to_string),
                    unit: r.unit().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();

    Some(OfferTerms {
        offer_token: first.offer_token().to_string(),
        offer_id: first.offer_id().map(str::to_string),
        legal_terms_url: terms
            .and_then(|t| t.legal_term())
            .and_then(|l| l.url())
            .map(str::to_string),
        refund_policy: terms
            .and_then(|t| t.support_term())
            .and_then(|s| s.refund_policy_description())
            .map(str::to_string),
        agreement_duration: terms
            .and_then(|t| t.validity_term())
            .and_then(|v| v.agreement_duration())
            .map(str::to_string),
        pricing,
    })
}

/// Whether an AWS error string indicates the Anthropic first-time-use form
/// hasn't been submitted for the account.
fn is_ftu_not_filled(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("ftuformnotfilled")
        || lower.contains("usecaseform")
        || lower.contains("use case details")
        || lower.contains("have not been submitted")
        || lower.contains("haven't been submitted")
        || lower.contains("not been submitted")
}

//! Cross-region inference profile parsing and geography translation.
//!
//! Pure helpers — no AWS calls. The caller is responsible for supplying the
//! list of profiles available in the target geography (via
//! `ListInferenceProfiles`).

use serde::{Deserialize, Serialize};

/// AWS Bedrock cross-region inference profile geographies Claria recognises.
///
/// Used as the prefix in inference profile IDs (e.g. `us.anthropic.claude-...`).
/// The list is closed because the grammar of profile IDs requires us to know
/// what counts as a scope. New geographies AWS adds in the future need a
/// release here — but this is for parsing only; the candidate set returned
/// to users always comes from live AWS discovery.
pub const KNOWN_GEOGRAPHIES: &[&str] = &["us", "eu", "apac", "ap-jp", "ap-au", "global"];

/// Parse an inference profile ID into its (geography, bare_model_id) parts.
///
/// Returns `None` for bare model IDs (e.g. `anthropic.claude-3-5-sonnet-...`),
/// application profile ARNs (`arn:aws:bedrock:...`), empty strings, and
/// anything else that doesn't match `geography.provider.model_id`.
///
/// Multipart geographies like `ap-jp` are parsed correctly: the first
/// dot-separated segment is the geography, even if it contains hyphens.
pub fn parse_inference_profile_id(s: &str) -> Option<(&str, &str)> {
    if s.is_empty() || s.starts_with("arn:") {
        return None;
    }

    let mut splitter = s.splitn(3, '.');
    let scope = splitter.next()?;
    let provider = splitter.next()?;
    let model = splitter.next()?;

    if !is_valid_scope_string(scope) {
        return None;
    }
    if provider.is_empty() || !provider.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    if model.is_empty() {
        return None;
    }

    let bare_start = scope.len() + 1;
    Some((scope, &s[bare_start..]))
}

fn is_valid_scope_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.split('-')
        .all(|g| !g.is_empty() && g.chars().all(|c| c.is_ascii_lowercase()))
}

fn looks_like_bare_model_id(s: &str) -> bool {
    if s.is_empty() || s.starts_with("arn:") {
        return false;
    }
    let parts: Vec<&str> = s.splitn(3, '.').collect();
    parts.len() == 2 && parts.iter().all(|p| !p.is_empty())
}

/// Foundation model lifecycle as the framework consumes it. Decoupled from
/// the AWS SDK enum so this module stays test-friendly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundationModelStatus {
    Active,
    Legacy { eol: jiff::Timestamp },
    Eol,
}

/// One inference profile available in the target geography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableProfile {
    pub profile_id: String,
    pub underlying_status: FoundationModelStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GeographyTranslation {
    SilentSwap { new_value: String },
    NoChangeNeeded,
    RequiresRepick { reason: RepickReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RepickReason {
    ProfileNotAvailableInGeography,
    UnderlyingModelLegacy { eol: jiff::Timestamp },
    UnderlyingModelEol,
    CurrentValueIsBareModelId,
    CurrentValueIsCustomApplicationProfile,
    CurrentValueMalformed,
}

/// Decide what should happen to the user's currently-stored profile ID when
/// they switch from one geography to another.
///
/// Pure function — caller supplies the list of profiles in the new geography
/// (typically from `ListInferenceProfiles`). Returns `SilentSwap` when the
/// constructed equivalent exists and is active, `NoChangeNeeded` when the
/// user is already in the target geography, and `RequiresRepick` otherwise.
pub fn translate_profile_for_geography(
    current: &str,
    new_geography: &str,
    available_profiles_in_new_geo: &[AvailableProfile],
) -> GeographyTranslation {
    if current.starts_with("arn:") {
        return GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueIsCustomApplicationProfile,
        };
    }

    if !KNOWN_GEOGRAPHIES.contains(&new_geography) {
        return GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueMalformed,
        };
    }

    let (current_geo, bare_model) = match parse_inference_profile_id(current) {
        Some(parsed) => parsed,
        None => {
            let reason = if looks_like_bare_model_id(current) {
                RepickReason::CurrentValueIsBareModelId
            } else {
                RepickReason::CurrentValueMalformed
            };
            return GeographyTranslation::RequiresRepick { reason };
        }
    };

    if current_geo == new_geography {
        return GeographyTranslation::NoChangeNeeded;
    }

    let constructed = format!("{new_geography}.{bare_model}");
    let profile = available_profiles_in_new_geo
        .iter()
        .find(|p| p.profile_id == constructed);

    match profile {
        None => GeographyTranslation::RequiresRepick {
            reason: RepickReason::ProfileNotAvailableInGeography,
        },
        Some(p) => match p.underlying_status {
            FoundationModelStatus::Active => GeographyTranslation::SilentSwap {
                new_value: constructed,
            },
            FoundationModelStatus::Legacy { eol } => GeographyTranslation::RequiresRepick {
                reason: RepickReason::UnderlyingModelLegacy { eol },
            },
            FoundationModelStatus::Eol => GeographyTranslation::RequiresRepick {
                reason: RepickReason::UnderlyingModelEol,
            },
        },
    }
}

//! Durable review findings for one report.
//!
//! A review sweep reads the accepted draft and emits [`Finding`] rows: a style
//! pass proposes an anchored text replacement the user can apply and undo, and
//! a consistency pass reports only. That asymmetry is structural, not a
//! convention — [`ReviewPass::Consistency`] with a [`StyleProposal`] fails
//! validation, so a consistency finding can never grow a write path.
//!
//! Findings are anchored to a [`FindingAnchor`], a `(section_id, revision)`
//! pair. Whether an anchor still describes the section is **derived** by
//! [`finding_is_stale`] rather than written by whoever edited the report; see
//! that function for why.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::CoreError,
    models::report::{ReportContent, validate_nonempty_text},
};
/// Findings cite records exactly the way a drafting run does, so they share the
/// one definition. Two `RecordCitation` types would also collide in the specta
/// bindings, which refuse duplicate exported names.
pub use crate::models::report_run::{MAX_CITATION_QUOTE_CHARACTERS, RecordCitation};

pub const REPORT_FINDINGS_SCHEMA_VERSION: u32 = 1;

/// Ceiling on findings retained for one report, resolved history included.
pub const MAX_REPORT_FINDINGS: usize = 200;

/// One review-pass coverage receipt per property, so a sweep that produced no
/// findings is still distinguishable from a sweep that never ran.
pub const MAX_REVIEW_COVERAGE: usize = 32;

/// Review property names are host-authored identifiers (`tense_drift`,
/// `unsupported_claim`), not model prose.
pub const MAX_FINDING_PROPERTY_CHARACTERS: usize = 64;

pub const MAX_FINDING_DESCRIPTION_CHARACTERS: usize = 1_000;

/// Record filenames are client-chosen and can be long; this only stops a
/// pathological value from being persisted.
const MAX_RECORD_FILENAME_CHARACTERS: usize = 1_024;

const MAX_MODEL_ID_CHARACTERS: usize = 200;

/// Every review finding recorded against one report, plus the coverage
/// receipts of the sweeps that produced them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct ReportFindings {
    pub schema_version: u32,
    pub report_id: Uuid,
    pub client_id: Uuid,
    pub findings: Vec<Finding>,
    pub coverage: Vec<ReviewCoverage>,
    #[specta(type = String)]
    pub updated_at: Timestamp,
}

/// One thing a review pass noticed about one section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct Finding {
    pub id: Uuid,
    pub pass: ReviewPass,
    /// The review property that raised this, e.g. `tense_drift`.
    pub property: String,
    /// The reviewing model, carried so applying a style proposal can credit
    /// the model that wrote the replacement.
    pub model_id: String,
    pub anchor: FindingAnchor,
    pub description: String,
    /// Where in the anchored section the finding points, resolved host-side
    /// from the model's quote.
    pub span: Option<TextSpan>,
    /// The passage this one contradicts. Consistency findings only in
    /// practice; nothing structural forbids a style finding from carrying one.
    pub conflicting: Option<ConflictingRef>,
    pub record_citation: Option<RecordCitation>,
    /// The anchored replacement a style finding offers. Never present on a
    /// consistency finding — see [`ReviewPass::Consistency`].
    pub proposal: Option<StyleProposal>,
    pub status: FindingStatus,
    /// The draft revision the applied replacement produced. Set only while
    /// the finding is [`FindingStatus::Applied`], so an undo can say which
    /// revision it is unwinding.
    pub applied_revision: Option<u64>,
    #[specta(type = Option<String>)]
    pub resolved_at: Option<Timestamp>,
    #[specta(type = String)]
    pub created_at: Timestamp,
}

/// The section and revision a finding describes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct FindingAnchor {
    pub section_id: Uuid,
    /// The section's authorship revision when the review read it.
    pub revision: u64,
}

/// A character range inside one block of the anchored section. Offsets are
/// `char` indices, not bytes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct TextSpan {
    /// Zero-based index into the section's blocks.
    pub block_index: u32,
    pub start_char: u32,
    pub end_char: u32,
}

/// The passage a finding conflicts with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct ConflictingRef {
    /// `None` means the conflicting passage is in the anchored section itself.
    pub section_id: Option<Uuid>,
    pub quote: String,
    pub span: Option<TextSpan>,
}

/// A style pass's anchored replacement: swap `original_text` for
/// `replacement_text` inside one paragraph of the anchored section.
///
/// The match is by text, not offset, and must be unique in the block — a
/// replacement that no longer matches, or matches twice, is refused rather
/// than guessed at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct StyleProposal {
    /// Zero-based index into the anchored section's blocks.
    pub block_index: u32,
    pub original_text: String,
    pub replacement_text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPass {
    /// Anchored, applicable replacements: tense, terminology, transitions,
    /// redundancy.
    Style,
    /// Read-only: contradictions, unsupported claims, cross-section conflicts.
    /// The pass has no write access, and the validator enforces it.
    Consistency,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    /// A style replacement the user applied. Reversible until the surrounding
    /// text changes.
    Applied,
    Dismissed,
    /// The anchored section moved on. Written lazily by the list path; the
    /// authoritative answer is always [`finding_is_stale`].
    Invalidated,
}

/// What the user chose to do with one finding.
///
/// This is the request shape, distinct from [`FindingStatus`], which records
/// where a finding ended up: undoing a finding puts it back to
/// [`FindingStatus::Open`], so the two do not correspond one to one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FindingAction {
    ApplyStyle,
    UndoStyle,
    Dismiss,
}

/// Proof that one review property actually ran over one revision, including
/// the case where it found nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct ReviewCoverage {
    pub pass: ReviewPass,
    pub property: String,
    pub model_id: String,
    /// The draft revision the pass read.
    pub revision: u64,
    pub sections_reviewed: u32,
    pub findings: u32,
    #[specta(type = String)]
    pub completed_at: Timestamp,
}

impl ReportFindings {
    /// An empty findings object, which is what a report that has never been
    /// reviewed has.
    pub fn new(client_id: Uuid, report_id: Uuid, now: Timestamp) -> Self {
        Self {
            schema_version: REPORT_FINDINGS_SCHEMA_VERSION,
            report_id,
            client_id,
            findings: Vec::new(),
            coverage: Vec::new(),
            updated_at: now,
        }
    }

    pub fn find(&self, finding_id: Uuid) -> Option<&Finding> {
        self.findings
            .iter()
            .find(|finding| finding.id == finding_id)
    }

    pub fn find_mut(&mut self, finding_id: Uuid) -> Option<&mut Finding> {
        self.findings
            .iter_mut()
            .find(|finding| finding.id == finding_id)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version > REPORT_FINDINGS_SCHEMA_VERSION {
            return Err(invalid(format!(
                "review findings schema version {} is newer than supported version {REPORT_FINDINGS_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if self.schema_version != REPORT_FINDINGS_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported review findings schema version {}",
                self.schema_version
            )));
        }
        if self.report_id.is_nil() || self.client_id.is_nil() {
            return Err(invalid("review findings IDs must not be nil"));
        }
        if self.findings.len() > MAX_REPORT_FINDINGS {
            return Err(invalid(format!(
                "a report may retain at most {MAX_REPORT_FINDINGS} review findings"
            )));
        }
        let mut ids = HashSet::with_capacity(self.findings.len());
        for finding in &self.findings {
            finding.validate()?;
            if !ids.insert(finding.id) {
                return Err(invalid(format!("duplicate finding ID {}", finding.id)));
            }
            if finding.created_at > self.updated_at {
                return Err(invalid(
                    "a review finding is newer than its findings object",
                ));
            }
        }

        if self.coverage.len() > MAX_REVIEW_COVERAGE {
            return Err(invalid(format!(
                "review coverage may contain at most {MAX_REVIEW_COVERAGE} rows"
            )));
        }
        let mut properties = HashSet::with_capacity(self.coverage.len());
        for coverage in &self.coverage {
            coverage.validate()?;
            if !properties.insert(coverage.property.as_str()) {
                return Err(invalid(format!(
                    "duplicate review coverage for property {}",
                    coverage.property
                )));
            }
        }
        Ok(())
    }
}

impl Finding {
    fn validate(&self) -> Result<(), CoreError> {
        if self.id.is_nil() {
            return Err(invalid("finding ID must not be nil"));
        }
        if self.anchor.section_id.is_nil() {
            return Err(invalid("finding anchor section ID must not be nil"));
        }
        validate_nonempty_text(
            "finding property",
            &self.property,
            MAX_FINDING_PROPERTY_CHARACTERS,
        )?;
        validate_nonempty_text("finding model ID", &self.model_id, MAX_MODEL_ID_CHARACTERS)?;
        validate_nonempty_text(
            "finding description",
            &self.description,
            MAX_FINDING_DESCRIPTION_CHARACTERS,
        )?;
        if let Some(span) = &self.span {
            span.validate()?;
        }
        if let Some(conflicting) = &self.conflicting {
            if conflicting.section_id.is_some_and(|id| id.is_nil()) {
                return Err(invalid("conflicting section ID must not be nil"));
            }
            validate_nonempty_text(
                "finding conflicting quote",
                &conflicting.quote,
                MAX_CITATION_QUOTE_CHARACTERS,
            )?;
            if let Some(span) = &conflicting.span {
                span.validate()?;
            }
        }
        if let Some(citation) = &self.record_citation {
            validate_nonempty_text(
                "finding citation filename",
                &citation.filename,
                MAX_RECORD_FILENAME_CHARACTERS,
            )?;
            validate_nonempty_text(
                "finding citation quote",
                &citation.quote,
                MAX_CITATION_QUOTE_CHARACTERS,
            )?;
        }

        // The consistency pass is read-only by construction. Refusing the
        // combination here means no review sweep, tool schema, or apply path
        // can hand a consistency finding a write.
        if self.pass == ReviewPass::Consistency && self.proposal.is_some() {
            return Err(invalid(
                "a consistency finding must not carry a replacement proposal",
            ));
        }
        if let Some(proposal) = &self.proposal {
            proposal.validate()?;
        }

        match self.status {
            FindingStatus::Applied => {
                if self.pass != ReviewPass::Style {
                    return Err(invalid("only a style finding can be applied"));
                }
                if self.applied_revision.is_none() {
                    return Err(invalid(
                        "an applied finding must record the revision it produced",
                    ));
                }
                if self.proposal.is_none() {
                    return Err(invalid(
                        "an applied finding must retain its replacement proposal",
                    ));
                }
            }
            FindingStatus::Open | FindingStatus::Dismissed | FindingStatus::Invalidated => {
                if self.applied_revision.is_some() {
                    return Err(invalid(
                        "only an applied finding may record an applied revision",
                    ));
                }
            }
        }
        // An unresolved finding has no resolution timestamp, and every
        // resolved one has exactly one. Undo therefore has to clear both
        // fields, not just the revision.
        match (self.status, self.resolved_at) {
            (FindingStatus::Open, Some(_)) => {
                Err(invalid("an open finding must not record a resolution time"))
            }
            (FindingStatus::Open, None) => Ok(()),
            (_, None) => Err(invalid("a resolved finding must record when")),
            (_, Some(resolved_at)) if resolved_at < self.created_at => {
                Err(invalid("a finding was resolved before it was created"))
            }
            (_, Some(_)) => Ok(()),
        }
    }
}

impl TextSpan {
    fn validate(&self) -> Result<(), CoreError> {
        if self.end_char <= self.start_char {
            return Err(invalid("a text span must end after it starts"));
        }
        Ok(())
    }
}

impl StyleProposal {
    fn validate(&self) -> Result<(), CoreError> {
        validate_nonempty_text(
            "style proposal original text",
            &self.original_text,
            MAX_CITATION_QUOTE_CHARACTERS,
        )?;
        validate_nonempty_text(
            "style proposal replacement text",
            &self.replacement_text,
            MAX_CITATION_QUOTE_CHARACTERS,
        )?;
        if self.original_text == self.replacement_text {
            return Err(invalid("a style proposal must change the text"));
        }
        Ok(())
    }
}

impl ReviewCoverage {
    fn validate(&self) -> Result<(), CoreError> {
        validate_nonempty_text(
            "review coverage property",
            &self.property,
            MAX_FINDING_PROPERTY_CHARACTERS,
        )?;
        validate_nonempty_text(
            "review coverage model ID",
            &self.model_id,
            MAX_MODEL_ID_CHARACTERS,
        )?;
        Ok(())
    }
}

/// Decode and validate persisted review findings. Validation is part of
/// decoding, exactly as it is for the workspace object, so a corrupt or
/// future-versioned object is never presented as review state.
pub fn decode_report_findings(bytes: &[u8]) -> Result<ReportFindings, CoreError> {
    let findings: ReportFindings = serde_json::from_slice(bytes)?;
    findings.validate()?;
    Ok(findings)
}

/// Whether `finding`'s anchor still describes the report.
///
/// Staleness is derived on every read and never written by a mutator. Nothing
/// that edits a report — a hand save, a proposal accept, a drafting run, an
/// applied replacement, a revert — has to remember to walk the findings and
/// invalidate them, so no mutation path can forget to. The
/// [`FindingStatus::Invalidated`] stamp the list path writes is a cache of
/// this answer for display; it is never the source of truth.
///
/// A finding is stale when its section is gone, or when the section's
/// authorship stamp has moved past the revision the review read. A section
/// with no authorship stamp has not been rewritten since the review, so it is
/// fresh.
pub fn finding_is_stale(finding: &Finding, content: &ReportContent) -> bool {
    match content
        .sections
        .iter()
        .find(|section| section.id == finding.anchor.section_id)
    {
        None => true,
        Some(section) => section
            .authorship
            .as_ref()
            .is_some_and(|authorship| authorship.revision > finding.anchor.revision),
    }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidFindings(message.into())
}

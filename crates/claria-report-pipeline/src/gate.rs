//! The completion gate: whether this report is finished, decided by code.
//!
//! Every other quality judgment in the pipeline is a model's opinion that a
//! human then accepts or rejects — a plan at the gate, a proposal in the
//! composer, a review finding in the pane. Completion is not one of those. The
//! clinician signs this document under their own license, and "an LLM approved
//! it" is not a thing anyone can sign under. So nothing here asks a model
//! anything: every check below is a decidable question about durable state,
//! answered by reading it.
//!
//! Six questions, each of which has a definite answer:
//!
//! | Check | Decided by |
//! |---|---|
//! | [`CompletionCheckKind::SectionNotTerminal`] | the run's per-section state machine |
//! | [`CompletionCheckKind::RequiredSectionEmpty`] | the plan's `required` rows against the saved draft |
//! | [`CompletionCheckKind::UnresolvedCitation`] | re-reading the cited record and searching for the quote |
//! | [`CompletionCheckKind::MissingCitation`] | a required section the writer drafted while citing nothing |
//! | [`CompletionCheckKind::PlaceholderText`] | the same marker scan the template import reports |
//! | [`CompletionCheckKind::UnresolvedFinding`] | open findings whose anchor still describes the section |
//!
//! The gate is **advisory for export**. It decides the explicit "complete"
//! status and the checklist the Writing pane renders; it does not stand
//! between a clinician and their own document, and a report that fails every
//! check still exports. Taking away an existing capability to enforce a new
//! opinion would be a regression dressed as rigor.
//!
//! Nothing in a [`CompletionCheck`] is PHI. Details are codes, counts, and
//! record filenames — never section text, never the quote that failed. The
//! report is returned to the frontend, logged next to audit events, and
//! copied into support bundles, so it carries only what a stranger may read.

use std::collections::{BTreeSet, HashMap, HashSet};

use aws_sdk_s3::Client as S3Client;
use claria_core::models::{
    findings::{FindingStatus, finding_is_stale},
    report::{
        ReportSection, ReportWorkspace, report_section_placeholder_count,
        report_text_placeholder_count,
    },
    report_run::{DraftRun, DraftRunStatus, RunSectionState},
};
use claria_report_store::{list_draft_runs, load_draft_run, load_for_report, load_report_findings};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ReportPipelineError,
    record_context::{RecordInventoryEntry, load_record_inventory},
    text::normalize_whitespace,
};

/// Everything the completion checklist knows about one report at one moment.
///
/// `checks` holds failures only, so an empty list and `complete` say the same
/// thing twice on purpose: the frontend renders the list, and the backend
/// answers the question.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct CompletionReport {
    pub complete: bool,
    /// Failures only — an empty list means the report is complete.
    pub checks: Vec<CompletionCheck>,
    /// The accepted draft revision these checks describe. A later edit makes
    /// the whole report stale, which is why the revision travels with it.
    pub evaluated_revision: u64,
    #[specta(type = String)]
    pub evaluated_at: jiff::Timestamp,
}

/// One reason the report is not complete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct CompletionCheck {
    pub kind: CompletionCheckKind,
    /// The section at fault, or `None` for a document-wide failure such as a
    /// placeholder left in the title.
    pub section_id: Option<Uuid>,
    /// PHI-free: state names, counts, and record filenames. Never section
    /// text, and never the quote that failed to resolve.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CompletionCheckKind {
    /// The drafting run never reached a decision about this section.
    SectionNotTerminal,
    /// The plan marked this section required and the saved draft has no body
    /// for it.
    RequiredSectionEmpty,
    /// A quote the writer attributed to a record is not in that record.
    UnresolvedCitation,
    /// A required section was drafted without citing anything.
    MissingCitation,
    /// Unresolved template markers survive in the saved draft.
    PlaceholderText,
    /// A review finding is still open against a section nobody has rewritten.
    UnresolvedFinding,
}

impl CompletionCheckKind {
    /// Sort rank, fixed by declaration order above. Two evaluations of the
    /// same state must produce byte-identical reports — the checklist is a
    /// document the clinician re-reads, and rows that reshuffle between two
    /// looks at an unchanged report read as things having changed.
    const fn rank(self) -> u8 {
        match self {
            Self::SectionNotTerminal => 0,
            Self::RequiredSectionEmpty => 1,
            Self::UnresolvedCitation => 2,
            Self::MissingCitation => 3,
            Self::PlaceholderText => 4,
            Self::UnresolvedFinding => 5,
        }
    }
}

/// Decide whether one report is complete.
///
/// Loads the accepted draft, the drafting run that produced it, and the
/// report's findings, then answers the six questions above against them. A
/// report with no run at all — hand-written, or imported and edited — skips
/// the run-dependent checks rather than failing them: there is no run to hold
/// anything against, and calling that incomplete would be an accusation the
/// gate cannot support.
pub async fn evaluate_report_completion(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<CompletionReport, ReportPipelineError> {
    let workspace = load_for_report(s3, bucket, client_id, report_id)
        .await?
        .workspace;
    let run = relevant_run(s3, bucket, client_id, report_id, &workspace).await?;
    let findings = load_report_findings(s3, bucket, client_id, report_id)
        .await?
        .findings;

    let mut checks = Vec::new();
    if let Some(run) = &run {
        checks.extend(section_state_checks(run));
        checks.extend(required_section_checks(run, &workspace));
        checks.extend(missing_citation_checks(run));
        checks.extend(citation_checks(s3, bucket, client_id, run).await?);
    }
    checks.extend(placeholder_checks(&workspace));
    checks.extend(
        findings
            .findings
            .iter()
            .filter(|finding| {
                finding.status == FindingStatus::Open
                    && !finding_is_stale(finding, &workspace.draft.content)
            })
            .map(|finding| CompletionCheck {
                kind: CompletionCheckKind::UnresolvedFinding,
                section_id: Some(finding.anchor.section_id),
                detail: finding.property.clone(),
            }),
    );

    sort_checks(&mut checks, &workspace);
    Ok(CompletionReport {
        complete: checks.is_empty(),
        checks,
        evaluated_revision: workspace.draft.revision,
        evaluated_at: jiff::Timestamp::now(),
    })
}

/// The run whose decisions the saved draft reflects, if there is one.
///
/// Two candidates, in order. A run the workspace still points at owns the
/// session — in progress, or stopped with durable partial state — and is what
/// the checklist is about. Otherwise the run that cut the current revision:
/// the newest one whose finish produced exactly the revision on the draft. A
/// run that built an older revision describes a document that has since moved
/// on, and holding its citations against the current text would be checking
/// the wrong claims.
async fn relevant_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    workspace: &ReportWorkspace,
) -> Result<Option<DraftRun>, ReportPipelineError> {
    if let Some(run_id) = workspace.active_run_id {
        let loaded = load_draft_run(s3, bucket, client_id, report_id, run_id).await?;
        return Ok(Some(loaded.run));
    }
    // Newest first, so the first match is the most recent finish.
    Ok(list_draft_runs(s3, bucket, client_id, report_id)
        .await?
        .into_iter()
        .find(|run| run.finalized_revision == Some(workspace.draft.revision)))
}

/// Sections the run left undecided. Only a run that has not finished can have
/// any: a completed run is one whose finisher already refused to cut a
/// revision until every planned section was written, skipped, or failed.
fn section_state_checks(run: &DraftRun) -> Vec<CompletionCheck> {
    if matches!(
        run.status,
        DraftRunStatus::Completed | DraftRunStatus::Abandoned
    ) {
        return Vec::new();
    }
    run.sections
        .iter()
        .filter_map(|section| {
            let state = match section.state {
                RunSectionState::Pending => "pending",
                RunSectionState::Drafting => "drafting",
                RunSectionState::Failed => "failed",
                RunSectionState::Drafted
                | RunSectionState::Flagged
                | RunSectionState::Skipped
                | RunSectionState::Kept => return None,
            };
            Some(CompletionCheck {
                kind: CompletionCheckKind::SectionNotTerminal,
                section_id: Some(section.section_id),
                detail: state.to_string(),
            })
        })
        .collect()
}

/// Required sections with nothing in them.
///
/// Read against the saved draft rather than the run, because the draft is what
/// gets signed: a section the run drafted and a later hand edit emptied is
/// empty, whatever the run remembers.
fn required_section_checks(run: &DraftRun, workspace: &ReportWorkspace) -> Vec<CompletionCheck> {
    let sections: HashMap<Uuid, &ReportSection> = workspace
        .draft
        .content
        .sections
        .iter()
        .map(|section| (section.id, section))
        .collect();
    run.plan
        .iter()
        .flat_map(|plan| plan.entries.iter())
        .filter(|entry| entry.required)
        .filter_map(|entry| {
            let detail = match sections.get(&entry.section_id) {
                None => "missing",
                Some(section) if section.skipped => "skipped",
                Some(section) if section.blocks.is_empty() => "empty",
                Some(_) => return None,
            };
            Some(CompletionCheck {
                kind: CompletionCheckKind::RequiredSectionEmpty,
                section_id: Some(entry.section_id),
                detail: detail.to_string(),
            })
        })
        .collect()
}

/// Required sections the writer drafted while citing nothing.
///
/// Skipped entirely for a synthetic plan. Those runs predate evidence
/// assignment — nobody ever asked the writer for citations, so the absence of
/// them says nothing about the section, and failing on it would tell the
/// clinician to go fix something the pipeline never requested.
fn missing_citation_checks(run: &DraftRun) -> Vec<CompletionCheck> {
    let Some(plan) = &run.plan else {
        return Vec::new();
    };
    if plan.synthetic && !plan.user_edited {
        return Vec::new();
    }
    let required: HashSet<Uuid> = plan
        .entries
        .iter()
        .filter(|entry| entry.required)
        .map(|entry| entry.section_id)
        .collect();
    run.sections
        .iter()
        .filter(|section| {
            required.contains(&section.section_id)
                && drafted_by_the_run(section.state)
                && section.citations.is_empty()
        })
        .map(|section| CompletionCheck {
            kind: CompletionCheckKind::MissingCitation,
            section_id: Some(section.section_id),
            detail: "no_citations".to_string(),
        })
        .collect()
}

/// Quotes the writer attributed to a record, re-checked against the record.
///
/// The gate re-reads the records rather than trusting the snapshot the run was
/// built from: the point of the check is that the quote is in the file *now*,
/// and a snapshot is exactly the thing that could have gone stale. One fetch
/// per distinct file, over the same bounded read path every other record
/// reader in this crate uses.
async fn citation_checks(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    run: &DraftRun,
) -> Result<Vec<CompletionCheck>, ReportPipelineError> {
    if run.finalized_revision.is_none() {
        // The run's sections are not what the draft says yet, so its citations
        // describe text nobody has accepted.
        return Ok(Vec::new());
    }
    let cited: BTreeSet<&str> = run
        .sections
        .iter()
        .filter(|section| drafted_by_the_run(section.state))
        .flat_map(|section| section.citations.iter())
        .map(|citation| citation.filename.as_str())
        .collect();
    if cited.is_empty() {
        return Ok(Vec::new());
    }

    let texts = load_cited_records(s3, bucket, client_id, &cited).await?;
    Ok(run
        .sections
        .iter()
        .filter(|section| drafted_by_the_run(section.state))
        .flat_map(|section| {
            section.citations.iter().filter_map(|citation| {
                let detail = match texts.get(citation.filename.as_str()) {
                    None => format!("missing_file:{}", citation.filename),
                    Some(text) => {
                        let quote = normalize_whitespace(&citation.quote);
                        if !quote.is_empty() && text.contains(&quote) {
                            return None;
                        }
                        format!("unmatched_quote:{}", citation.filename)
                    }
                };
                Some(CompletionCheck {
                    kind: CompletionCheckKind::UnresolvedCitation,
                    section_id: Some(section.section_id),
                    detail,
                })
            })
        })
        .collect())
}

/// Whitespace-normalized text for every cited file that can still be read.
/// A file missing from the map is one the gate could not read at all — gone,
/// oversized, or no longer decodable as text — and every quote against it
/// fails as `missing_file`.
async fn load_cited_records(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    cited: &BTreeSet<&str>,
) -> Result<HashMap<String, String>, ReportPipelineError> {
    let inventory: HashMap<String, RecordInventoryEntry> =
        load_record_inventory(s3, bucket, client_id)
            .await
            .map_err(|source| {
                ReportPipelineError::storage(
                    "reading the client's records for the completion check",
                    source,
                )
            })?
            .into_iter()
            .map(|entry| (entry.filename.clone(), entry))
            .collect();

    // Resolved to owned entries first, and moved into each future rather than
    // borrowed: an async block that borrows its closure argument stops being
    // higher-ranked enough once this call sits under a Tauri command.
    let mut entries: Vec<RecordInventoryEntry> = Vec::with_capacity(cited.len());
    for filename in cited {
        if let Some(entry) = inventory.get(*filename) {
            entries.push(entry.clone());
        }
    }
    let fetches = entries.into_iter().map(|entry| async move {
        if entry.source_bytes > claria_core::record_text::MAX_RECORD_TEXT_BYTES {
            return Ok(None);
        }
        let output = claria_storage::objects::get_object_bounded(
            s3,
            bucket,
            &entry.read_key,
            claria_core::record_text::MAX_RECORD_TEXT_BYTES,
        )
        .await?;
        Ok::<_, claria_storage::error::StorageError>(
            claria_core::record_text::decode_record_text(&output.body)
                .map(|text| (entry.filename.clone(), normalize_whitespace(text))),
        )
    });

    let loaded = stream::iter(fetches)
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut texts = HashMap::new();
    for record in loaded {
        let record = record.map_err(|source| {
            ReportPipelineError::storage("reading a cited record for the completion check", source)
        })?;
        if let Some((filename, text)) = record {
            texts.insert(filename, text);
        }
    }
    Ok(texts)
}

/// Unresolved template markers left in the saved draft.
///
/// The same scan the DOCX import reports at template time, run again on what
/// is about to be signed — a heading of `[CLIENT NAME]` is the failure mode
/// this whole gate exists for. Skipped sections are exempt: they are excluded
/// from the export, so their carryover reaches nobody.
fn placeholder_checks(workspace: &ReportWorkspace) -> Vec<CompletionCheck> {
    let content = &workspace.draft.content;
    let mut checks = Vec::new();
    let title = report_text_placeholder_count(&content.title);
    if title > 0 {
        checks.push(CompletionCheck {
            kind: CompletionCheckKind::PlaceholderText,
            section_id: None,
            detail: title.to_string(),
        });
    }
    checks.extend(
        content
            .sections
            .iter()
            .filter(|section| !section.skipped)
            .filter_map(|section| {
                let count = report_section_placeholder_count(section);
                (count > 0).then(|| CompletionCheck {
                    kind: CompletionCheckKind::PlaceholderText,
                    section_id: Some(section.id),
                    detail: count.to_string(),
                })
            }),
    );
    checks
}

/// The run put words in this section. `Flagged` is a drafted section a review
/// pass raised something against, which is why the resume planner treats the
/// two alike.
const fn drafted_by_the_run(state: RunSectionState) -> bool {
    matches!(state, RunSectionState::Drafted | RunSectionState::Flagged)
}

/// Order the checklist by kind, then by where in the document the section sits.
///
/// Sections are ordered by their position in the saved draft rather than by
/// the run's own `position`, so the list reads top to bottom the way the
/// document does. The remaining keys exist only to make the order total: two
/// checks of the same kind against the same section must not be free to swap.
fn sort_checks(checks: &mut [CompletionCheck], workspace: &ReportWorkspace) {
    let order: HashMap<Uuid, usize> = workspace
        .draft
        .content
        .sections
        .iter()
        .enumerate()
        .map(|(index, section)| (section.id, index))
        .collect();
    checks.sort_by(|left, right| {
        let position = |check: &CompletionCheck| {
            check
                .section_id
                .map_or(0, |id| order.get(&id).map_or(usize::MAX, |index| index + 1))
        };
        left.kind
            .rank()
            .cmp(&right.kind.rank())
            .then_with(|| position(left).cmp(&position(right)))
            .then_with(|| left.section_id.cmp(&right.section_id))
            .then_with(|| left.detail.cmp(&right.detail))
    });
}

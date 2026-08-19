//! The seven per-property review instructions: the only bytes that differ
//! between the branches of one fan-out.
//!
//! An instruction is two halves, and only one of them is the user's.
//!
//! - The **body** is the checklist that says what the property *is* — the
//!   numbered list of things to look at. It is what a clinician has an opinion
//!   about ("also check the referral question is restated the same way"), and
//!   it is what the preflight list lets them edit or drop before a sweep
//!   fires.
//! - The **frame** is the host's contract with its own validator: the coverage
//!   rule, the verbatim-quote rule, the style/consistency split on whether a
//!   replacement may be proposed, and the forced single call to
//!   `submit_review_rows` under this property's name. Every one of those
//!   sentences has a check behind it that rejects the answer if it is not
//!   obeyed, so it is composed around whatever body the caller supplied and is
//!   not editable. A pass whose contract a user could delete would fail
//!   validation, cost its repair round, and leave the property with no
//!   coverage row — a worse outcome than not being able to edit it.
//!
//! Everything above these — the analysis policy, the record corpus, the
//! template structure, the drafted sections — is byte-identical across all
//! seven requests and sits above a cache checkpoint. These blocks are what
//! each branch pays for, so each is a few hundred tokens and no more.
//!
//! Every one of them enumerates what to check. "Flag any gaps" produces a
//! pass that finds whatever the model happened to notice on the way past;
//! "for every factual assertion, verify a supporting span exists in the record
//! corpus" produces a pass whose coverage a clinician can reason about. Each
//! also demands the explicit no-issues row, because a review that reports
//! nothing about a section and a review that never read it are the same bytes
//! otherwise.
//!
//! No instruction names another property. The fan-out's test harness routes a
//! scripted response to a branch by finding its property name in the request,
//! and a cross-reference would make two branches indistinguishable.

use claria_bedrock::analysis::ReviewProperty;

/// The shipped checklist for one property: the editable half of its
/// instruction, and the value the preflight list defaults every pass to.
pub(crate) const fn default_body(property: ReviewProperty) -> &'static str {
    match property {
        ReviewProperty::TenseDrift => TENSE_DRIFT,
        ReviewProperty::Terminology => TERMINOLOGY,
        ReviewProperty::Transitions => TRANSITIONS,
        ReviewProperty::Redundancy => REDUNDANCY,
        ReviewProperty::InternalContradiction => INTERNAL_CONTRADICTION,
        ReviewProperty::UnsupportedClaim => UNSUPPORTED_CLAIM,
        ReviewProperty::CrossSectionConflict => CROSS_SECTION_CONFLICT,
    }
}

/// The instruction for one branch of the fan-out: the host's fixed frame
/// composed around `body`.
pub(crate) fn instruction(property: ReviewProperty, body: &str) -> String {
    let name = property.as_str();
    let closing = if property.is_style() {
        "Attach a replacement to every finding: the exact text to find, copied from the section \
         and unique within it, and the text to put in its place. Change only what this property \
         is about — a replacement that also rewrites a clinical fact will be rejected by the \
         clinician reading it."
    } else {
        "Propose no text. This property reports; it does not edit. A finding that carries a \
         replacement is rejected outright and costs this pass its correction round."
    };
    format!(
        "You are now REVIEWING the finished draft below. You are not planning it and not \
         rewriting it: you are reading one property across the whole document and reporting what \
         you find.\n\n\
         The property for this pass is {name}. Report nothing else. Another pass is reading every \
         other property, and a finding filed here under a different concern is a finding the \
         clinician sees twice.\n\n\
         {body}\n\n\
         Coverage is the point of this pass. Return exactly one row per section in \
         <untrusted_draft_sections>, in the order that block lists them, copying each section_id \
         exactly. A section where you find nothing gets a row with status \"no_issues\" and an \
         empty findings array — that row is the evidence the section was read, and omitting it \
         fails validation.\n\n\
         A section may have been drafted under a clinician-set record restriction: its writer was \
         deliberately given a subset of the corpus and could not have read the rest. You are not \
         told which sections those are. So where a finding amounts to \"this section is thinner \
         than the records allow\" or \"this omits what the corpus has\", file it, but say in the \
         detail that the drafter may have been restricted to a subset — that turns the row into a \
         question about the section's sources, which the clinician can answer, instead of an \
         accusation about the writer, which they cannot.\n\n\
         Quote from the section you are filing against, character for character. The host \
         searches that section's own text for your quote, treating any run of whitespace as \
         equal to any other; a quote you have tidied, shortened, or restated resolves against \
         nothing and the finding is discarded.\n\n\
         {closing}\n\n\
         Call submit_review_rows exactly once, with property set to {name}.\n"
    )
}

const TENSE_DRIFT: &str = "\
Check, for every section: \n\
1. Which tense the section's narrative body settles into — past for what was done and observed, \
present for what currently stands. Report each sentence that departs from it.\n\
2. Whether test administration, observation, and interview are described in the same tense \
throughout the section. Report each one that switches.\n\
3. Whether a sentence changes tense inside itself between its clauses.\n\
4. Whether recommendations and current status are written in the present or future while the \
history around them is written in the past, and whether that boundary is consistent across the \
document.\n\
Do not report a tense that is correct because the sentence genuinely describes a different time.";

const TERMINOLOGY: &str = "\
Check, for every section: \n\
1. Every instrument, scale, and subtest name against the form used the first time the document \
names it. Report each later mention that abbreviates, expands, or renames it differently.\n\
2. Every way the document refers to the person it is about — name, role, relationship, pronoun. \
Report each inconsistency.\n\
3. Every diagnostic or descriptive label that appears in more than one form (a construct named \
one way here and another way there).\n\
4. Every acronym, for whether it is expanded at first use and then used consistently.\n\
5. Every score type named (standard score, scaled score, percentile, T-score), for whether the \
same type is named the same way each time.";

const TRANSITIONS: &str = "\
Check, for every section: \n\
1. The opening sentence, for whether it states what the section is about rather than continuing a \
thought from a section the reader may not have read in order.\n\
2. Each paragraph boundary inside the section, for a jump between topics with nothing carrying \
the reader across it.\n\
3. Each place the document moves from data to interpretation, for whether the move is signalled \
rather than implied by adjacency.\n\
4. Sentences that begin with a connective (however, therefore, additionally) whose logical \
relation does not hold between what precedes and what follows.\n\
5. Lists and enumerations that begin without a sentence saying what is being listed.";

const REDUNDANCY: &str = "\
Check, for every section: \n\
1. Sentences that restate, in different words, something already stated in the same section.\n\
2. A finding, score, or history detail given in full in one section and given in full again in \
another, where the second is not doing interpretive work the first did not.\n\
3. Phrases that add length without adding meaning (\"it should be noted that\", \"in terms of\", \
\"the results of the assessment indicated that\").\n\
4. Qualifiers stacked on one claim (\"appears to possibly suggest\").\n\
Report the later or weaker instance, not the first statement of the fact, and never propose \
removing a fact — propose the tighter wording that keeps it.";

const INTERNAL_CONTRADICTION: &str = "\
Check, for every section, every statement that could be contradicted by another statement in the \
same document, and report each pair that is: \n\
1. Every score, index, and percentile against every other mention of that same score.\n\
2. Every date, age, and grade level against every other mention of the same event.\n\
3. Every statement about what the person can or cannot do against every other statement about \
the same ability.\n\
4. Every summary or conclusion against the specific findings it summarizes — a conclusion that \
asserts more, less, or otherwise than the data section it rests on.\n\
5. Every statement of history (who reported what, when) against every other statement about the \
same event.\n\
For each contradiction, anchor the finding on one of the two passages and put the other in \
conflicting_span, naming its section_id when it is in a different section.";

const UNSUPPORTED_CLAIM: &str = "\
For every factual assertion in every section, verify that a supporting span exists in the record \
corpus in <untrusted_record_context>, and report each assertion for which one does not. Work \
through them in this order: \n\
1. Every numeric result — score, percentile, index, age, date, duration, count.\n\
2. Every attributed statement (\"the parent reported\", \"the teacher observed\"), for whether \
the record shows that person saying it.\n\
3. Every statement about history — prior diagnoses, prior services, prior testing, medical events.\n\
4. Every statement about the current setting, placement, or supports in place.\n\
5. Every characterization of behaviour presented as observed rather than inferred.\n\
An assertion the corpus supports only in part is unsupported: report the part that is not there. \
Put the supporting span in record_citation when one exists but says something narrower than the \
draft claims. Clinical interpretation drawn openly from stated data is not an unsupported claim; \
a fact with no source is.";

const CROSS_SECTION_CONFLICT: &str = "\
Check every section against every other section, and report each pair that cannot both be true or \
cannot both be the document's position: \n\
1. A fact stated one way in one section and another way in another (a score, a date, an age, a \
name, a setting).\n\
2. A conclusion in a summary or recommendation section that the section it draws on does not \
support, or that a different section undercuts.\n\
3. A recommendation that assumes a circumstance another section says is not the case.\n\
4. A statement of severity, frequency, or impairment given differently in two sections.\n\
5. A referral question stated in one section and answered in another as though it had been a \
different question.\n\
Anchor each finding on the passage in the section you are filing the row against, and put the \
other passage in conflicting_span with its own section_id.";

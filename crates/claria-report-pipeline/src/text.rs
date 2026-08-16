//! The one whitespace rule every host-side quote check is decided by.
//!
//! Three passes compare a model's quote against text it was shown — the
//! planner resolving evidence against the record corpus, a review branch
//! anchoring a finding inside a section, and the completion gate re-checking a
//! citation against the record itself. All three have to agree on what
//! "verbatim" means, because a quote the planner accepted and the gate then
//! rejected would be a contradiction the clinician could do nothing about.
//!
//! The rule: collapse every run of whitespace to one space and trim the ends,
//! on both sides, before searching. A model copying a span out of a wrapped
//! clinical note reliably reflows its line breaks and nothing else. Anything
//! beyond whitespace — a corrected typo, a trimmed clause, a paraphrase —
//! still fails, which is the point.

/// Collapse every run of whitespace in `text` to one space and trim the ends.
pub(crate) fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

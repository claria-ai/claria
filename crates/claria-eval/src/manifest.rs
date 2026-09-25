//! The writer-eval fixture's expectations, as committed beside its records.
//!
//! `fixtures/writer-eval/manifest.json` is generated from the same in-script
//! data model as the records it describes, so the two cannot drift. This module
//! only reads it. Field names are the on-disk contract.

use std::{collections::BTreeSet, path::Path};

use eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};

/// What the fixture expects of a plan and a drafted report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Always true in a committed fixture. Checked rather than assumed, because
    /// a corpus of clinical paperwork that is *not* marked synthetic has no
    /// business in this repository.
    pub synthetic: bool,
    pub client_name: String,
    pub client_date_of_birth: String,
    pub records: Vec<RecordExpectation>,
    pub sections: Vec<SectionExpectation>,
    pub sentinels: Vec<Sentinel>,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordExpectation {
    pub filename: String,
    pub kind: String,
    pub date_range: Vec<String>,
    pub topics: Vec<String>,
    #[serde(default)]
    pub facts: Vec<Fact>,
}

/// One unique string a record carries, and the section that owns it.
///
/// Uniqueness across the corpus is what makes a grep meaningful: a token found
/// in a drafted section came from exactly one record, so "this section carries
/// a fact another section owns" is decidable without reading the prose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub token: String,
    pub owning_section: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionExpectation {
    pub heading: String,
    pub expected_intent: ExpectedIntent,
    #[serde(default)]
    pub must_evidence: Vec<String>,
    #[serde(default)]
    pub may_evidence: Vec<String>,
    #[serde(default)]
    pub must_not_evidence: Vec<String>,
    #[serde(default)]
    pub must_facts: Vec<String>,
    #[serde(default)]
    pub must_not_facts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedIntent {
    Draft,
    Skip,
}

/// A record planted to be selected by mistake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sentinel {
    pub filename: String,
    pub kind: String,
    #[serde(default)]
    pub purpose: Option<String>,
    /// Sections that must not name this file as evidence. Empty means the file
    /// is legitimate evidence somewhere and only its tokens are policed.
    #[serde(default)]
    pub never_evidence_for: Vec<String>,
    /// Strings that must not appear anywhere in the drafted report.
    #[serde(default)]
    pub tokens_never_in_output: Vec<String>,
}

/// Two records that disagree, and the section that has to surface it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub topic: String,
    pub section: String,
    pub records: Vec<String>,
    pub tokens: Vec<String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .wrap_err_with(|| format!("reading the fixture manifest at {}", path.display()))?;
        let manifest: Self = serde_json::from_slice(&bytes)
            .wrap_err_with(|| format!("parsing the fixture manifest at {}", path.display()))?;
        if !manifest.synthetic {
            return Err(eyre!(
                "{} is not marked synthetic; refusing to treat a corpus of real \
                 clinical records as an eval fixture",
                path.display()
            ));
        }
        Ok(manifest)
    }

    pub fn section(&self, heading: &str) -> Option<&SectionExpectation> {
        self.sections.iter().find(|s| s.heading == heading)
    }

    /// Every fact token in the corpus, paired with the section that owns it.
    pub fn facts(&self) -> impl Iterator<Item = &Fact> {
        self.records.iter().flat_map(|record| record.facts.iter())
    }

    /// The tokens a section is supposed to carry, drawn from the records rather
    /// than from the section's own `must_facts`, so "a fact another section
    /// owns" has a corpus-wide definition.
    pub fn tokens_owned_by(&self, heading: &str) -> BTreeSet<&str> {
        self.facts()
            .filter(|fact| fact.owning_section == heading)
            .map(|fact| fact.token.as_str())
            .collect()
    }

    /// Tokens owned by some other section than `heading`.
    pub fn tokens_owned_elsewhere(&self, heading: &str) -> BTreeSet<&str> {
        self.facts()
            .filter(|fact| fact.owning_section != heading)
            .map(|fact| fact.token.as_str())
            .collect()
    }
}

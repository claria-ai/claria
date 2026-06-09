//! claria-live-aws
//!
//! Framework for AWS catalog values whose source of truth is AWS itself,
//! not Claria's source code. Providers discover values at runtime (read-only)
//! and the framework surfaces drift as a plan the user approves.
//!
//! Two flavors of provider:
//! - `UserChoice` — drift resolved by writing to local config (scoped creds)
//! - `ProvisionerContributor` — drift resolved by contributing `ResourceSpec`s
//!   into the provisioner manifest (apply runs under elevation through the
//!   existing provisioner pipeline)
//!
//! See `crates/claria-live-aws/README.md` and the CLAUDE.md recipe for how
//! to add a new live AWS value.

pub mod error;
pub mod provider;
pub mod registry;

pub use crate::error::ProviderError;
pub use crate::provider::{
    Candidate, ChangeReason, LiveAwsValueProvider, ProviderKind, Reconciliation,
};
pub use crate::registry::{
    LiveAwsValueRegistry, ReconciliationReport, UserChoiceDriftEntry,
};

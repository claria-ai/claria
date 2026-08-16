//! Headless eval harness for the writer pipeline.
//!
//! `claria-eval` drives the plan → gate → parallel-draft path against a real
//! AWS environment with no UI, so an agent can exercise Bedrock and read the
//! results — progress events with elapsed timestamps, token counts, cost, and
//! OTLP traces — without a human clicking through the desktop app.
//!
//! # Blessed exception to the config boundary rule
//!
//! `claria-desktop` is normally the only crate that reads local config files.
//! This tool breaks that rule deliberately and in one direction only: it
//! **reads** the desktop's `config.json` and never writes it. It deserializes
//! its own minimal struct ([`config::EvalConfig`]) covering just the region,
//! system name, account ID, and credential source, ignoring every other
//! field, and it runs no migrations. That means it can lag a `config_version`
//! bump — if a migration renames one of those five fields, this tool starts
//! failing to parse and needs the same rename. Running the desktop app once
//! after an upgrade migrates the file on disk, after which this tool reads the
//! migrated shape.
//!
//! The only file this tool writes is its own spend state (see [`governor`]).
//!
//! # PHI
//!
//! Plans and drafts are printed to stdout in full — that is what the harness
//! is for, and it is pointed at the smoke-test environment. Nothing derived
//! from report or record content ever reaches a span attribute or a log field:
//! telemetry carries UUIDs, counts, model IDs, durations, and dollars only.

pub mod config;
pub mod cost;
pub mod governor;
pub mod pipeline;
pub mod preferences;
pub mod progress;
pub mod telemetry;

/// The AWS handles and bucket every S3-touching subcommand needs, mirroring
/// the desktop's per-command context.
pub struct EvalContext {
    pub sdk_config: aws_config::SdkConfig,
    pub s3: aws_sdk_s3::Client,
    pub bucket: String,
}

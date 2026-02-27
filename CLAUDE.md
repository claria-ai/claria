# Claria

## Error Handling
- `thiserror` in every lib crate — one error enum per crate (e.g., `StorageError`, `SearchError`)
- `eyre` in bin crates (`claria-lambda`, `claria-desktop`)
- `color-eyre` in `claria-desktop` for development
- No `unwrap()` outside of tests

## Naming
- Standard Rust: `snake_case` modules/functions, `CamelCase` types, `SCREAMING_SNAKE` constants
- `snake_case` for all JSON serialization (no camelCase)

## Serialization
- `serde` with `#[serde(rename_all = "snake_case")]` on enums
- All `pub` types derive `Serialize` and `Deserialize`

## Date/Time
- `jiff` for all date/time handling (not `chrono`)

## Testing
- Tests live in `tests/` directory, not inline `mod tests`

## Dependencies
- Pin exact versions (e.g., `serde = "=1.0.219"`)
- Commit `Cargo.lock`

## Code Style
- Nightly `rustfmt` with `imports_granularity = "Crate"`
- Clippy warnings are errors: `cargo clippy -- -D warnings`

## Git
- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Never squash
- Descriptive kebab-case branch names

## Architecture

### Design Principles
- Small, focused, testable crates — each crate has one job
- Maximise parallel compilation by keeping the dependency graph wide, not deep
- Auditability: every AWS action is traceable to a crate and function
- Discoverability: if you're looking for how X works, there's one obvious crate to look in
- No `unwrap()` outside of tests
- All `pub` types get `Serialize`/`Deserialize`

### Crate Responsibilities (MVC)

**`claria-desktop` — Controller + View**
- Tauri app: UI rendering, user flow orchestration, config persistence
- Knows *what* to ask the user and *when* to call into library crates
- Never contains business logic, IAM policies, sync algorithms, or AWS service knowledge
- Translates user input into `SdkConfig` and passes it to library crates
- Persists results (config, state) to local disk
- Teaches the user about the cloud, HIPAA, and their responsibilities

**`claria-provisioner` — Model (AWS account intelligence)**
- The "brains" of how an AWS account is configured
- Credential classification: detect root / admin / scoped / insufficient
- Account bootstrap: create least-privilege IAM users and policies from broad credentials
- Resource provisioning: scan → plan → execute for S3, CloudTrail, Bedrock
- Never reads/writes local config — returns structured results for the caller to persist

**`claria-storage` — S3 object operations**
- CRUD for objects in S3 (get, put, delete, list, presign)
- No knowledge of what the objects represent (cases, reports, etc.)

**`claria-search` — Full-text search**
- Local Tantivy index with S3 backup/restore

**`claria-bedrock` — LLM interactions**
- Bedrock API calls for report generation and analysis

**`claria-audit` — Audit trail**
- Structured audit event logging

**`claria-core` — Shared types**
- Domain types shared across multiple crates

### Boundary Rules
- Library crates accept `&aws_config::SdkConfig` — they never build their own SDK configs
- Library crates return `Result<T, CrateError>` — the caller decides how to present errors
- Library crates never do I/O to the local filesystem (except `claria-search` for its index)
- `claria-desktop` is the only crate that reads/writes local config files
- Crates communicate through well-defined public APIs, not shared mutable state

## Config Versioning

`config.json` carries a `config_version` field (u32). Current version: **1**.

### Rules
- Every schema change to `ClariaConfig` (new field, renamed field, changed type) bumps `CURRENT_VERSION` in `config.rs`
- Each bump gets a migration function in `migrate()` that transforms the raw JSON from version N to N+1
- Migrations are pure `serde_json::Value` transforms — no async, no network, no filesystem beyond the config itself
- Async backfills (e.g. resolving `account_id` via STS) live in the Tauri command layer (`commands.rs`), not in migrations
- `save_config` always stamps `config_version = CURRENT_VERSION`
- `load_config` reads raw JSON, runs migrations in order, then deserializes into `ClariaConfig`
- If `config_version` on disk is higher than `CURRENT_VERSION`, `load_config` returns an error telling the user to update
- New fields must use `#[serde(default)]` so that pre-migration JSON still deserializes during the migration window
- Never delete a migration — the chain must be able to upgrade from v0 to current in one pass

### Adding a new version
1. Bump `CURRENT_VERSION` in `config.rs`
2. Add `#[serde(default)]` on any new fields in `ClariaConfig`
3. Add `if from_version < N { ... }` block in `migrate()` that sets the new field and stamps `config_version = N`
4. If the field needs async backfill, add logic in `load_config` command in `commands.rs`

## Claude Code
- Run `cargo check` after medium and larger edits
- Run `cargo test` before committing
- Run `cargo clippy -- -D warnings` before committing

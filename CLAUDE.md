# Claria

A desktop app for interacting with AWS S3/Bedrock. Used for HIPAA compliant generative AI.

No custom API, just direct Desktop -> AWS via AWS Rust SDK authentication.

## Error Handling
- `thiserror` in every lib crate — one error enum per crate (e.g., `StorageError`, `SearchError`)
- `eyre` in bin crates (`claria-desktop`, `claria-mock-aws`)
- `color-eyre` in `claria-desktop` for development
- No `unwrap()` outside of tests
- Never just swallow an error, bubble it up untils exposed in the UI

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
- Create a commit after any batch of changes is done
- Every meaningful commit (or grouping of commits with a common theme) should get noted in the CHANGELOG.md under a `## [Unreleased]` section header. `cargo release` rewrites this header to `## [version] — date` via `pre-release-replacements` in Cargo.toml
- CHANGELOG entries are **headline-style**: one bullet = one fact, in plain prose. Trust the reader to consult the diff for the why and the how. Don't use bolded lead-ins, don't name structs/files/commands inline, don't justify the choice or document rejected alternatives, don't hedge. If a bullet runs more than two lines it's probably two bullets, or it's saying too much.

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
- CRUD for objects in S3 (get, put, delete, list)
- No knowledge of what the objects represent (cases, reports, etc.)
- `audit` module: structured audit events and the durable one-object-per-event S3 sink (the audit trail is S3 writes, which this crate owns)

**`claria-records` — Client records**
- Client CRUD, name history, optimistic-concurrency rename, and record content search
- Record inventory: the S3 walk behind the sidecar-visibility rules (the pure rule itself lives in `claria-core`'s `s3_keys.rs`)
- ETag-revalidated read-through cache for record objects
- Retryable, compensating client delete/restore lifecycle
- Depends on `claria-report-store` (lifecycle restores report workspaces), so the report crates must never depend on this one

**`claria-report-pipeline` — Report-writing orchestration**
- Runs one writing request end to end: prompt composition, the Bedrock tool loop, bounded record reads, proposal staging, and the whole-document draft protocol

**`claria-report-store` — Durable writer state**
- Workspace objects and their optimistic-concurrency protocol, immutable revisions, attempt and per-call usage receipts
- The global writer prompt and writer template libraries
- No Bedrock knowledge: callers hand it fully built records

**`claria-bedrock` — LLM interactions**
- Bedrock API calls for chat, text extraction, and translation

**`claria-billing` — Cost Explorer + Bedrock pricing**
- Wraps AWS Cost Explorer (`GetCostAndUsage`) and owns the Bedrock per-token pricing table
- `PRICING_VERSION` is stamped onto every captured `TurnUsage` so historical costs never shift

**`claria-transcribe` — Audio transcription**
- Runs imported recordings through Amazon Transcribe (standard or medical), including speaker and language options
- Runs Record Memo locally through `transcribe.cpp` and curated GGUF Whisper models
- Processes Record Memo PCM on-device and supports Metal acceleration with CPU fallback
- Owns the stable transcript sidecar parser and renderer

**`claria-core` — Shared types**
- Domain types shared across multiple crates
- `s3_keys.rs` is the single source of truth for all S3 object paths

**`claria-mock-aws` — Fake AWS for tests**
- axum server that speaks the S3, STS, IAM, Bedrock, CloudTrail, Cost Explorer, Transcribe, and Artifact wire protocols
- Runs as a standalone binary or in-process via `testing::MockServer`, so tests drive real AWS SDK clients against an ephemeral port

### Boundary Rules
- Library crates accept `&aws_config::SdkConfig` — they never build their own SDK configs
- Library crates return `Result<T, CrateError>` — the caller decides how to present errors
- Library crates never do I/O to the local filesystem. Blessed exception: the provisioner's dual-write state persistence keeps a local safety-net copy — but it receives the state directory from the desktop caller (`build_persistence` parameter) and never derives a path itself
- `claria-desktop` is the only crate that reads/writes local config files (and the only crate that knows where local app directories live)
- Crates communicate through well-defined public APIs, not shared mutable state

## S3 Key Layout

All S3 object paths are defined in `claria-core/src/s3_keys.rs`. Key prefixes:

| Path pattern | What it holds |
|---|---|
| `clients/{uuid}.json` | Client record JSON |
| `records/{uuid}/{filename}` | Files attached to a client |
| `records/{uuid}/{filename}.text` | Sidecar with extracted text (hidden in UI when base file exists) |
| `records/{uuid}/chat-history/{chat_id}.json` | Persisted, user-named chat sessions |
| `report-authoring/{uuid}/workspace.json` | Accepted report, named writer session, and proposal history |
| `report-authoring/{uuid}/attempts/` | Bounded writer-attempt diagnostics and usage |
| `report-authoring/{uuid}/templates/{sha256}.docx` | Immutable redacted template snapshot used to preserve Word formatting on export |
| `writer_templates/{template_uuid}.docx` | Global managed, redacted writer-template source |
| `writer_templates/{template_uuid}.json` | Writer-template metadata (name, size, upload date) |
| `writer_templates/{template_uuid}.usage.json` | Best-effort writer-template use count |
| `claria-prompts/system-prompt.md` | Custom chat system prompt |
| `claria-prompts/pdf-extraction.md` | Custom PDF/DOCX extraction prompt |
| `claria-prompts/report-system-prompt.md` | Custom writer prompt body (targeted edits); fixed trust rules are always appended |
| `claria-prompts/full-report-system-prompt.md` | Custom whole-report prompt body; fixed trust rules are always appended |
| `claria-prompts/writer-library/{prompt_uuid}.json` | Saved writer steering prompt that prefills the instruction box |
| `_cloudtrail/` | CloudTrail audit logs |
| `_transcribe/{job_name}.json` | Amazon Transcribe job output, read once then deleted |
| `_state/provisioner.json` | Provisioner state |
| `_state/preferences.json` | Synced user preferences |

### Sidecar Pattern
Binary uploads generate a `.text` sidecar alongside the original: PDF and DOCX sidecars contain structured Markdown, while audio sidecars contain transcripts. The file list hides sidecars when the base file exists. Printable UTF-8 originals—including Markdown, JSON, CSV, source files, and extensionless notes—are read directly without renaming or conversion; content validation, not the filename extension, determines whether an original is text.

## IAM Action Names

The IAM policy in `account_setup.rs` uses **IAM action names**, which sometimes differ from S3 API operation names. The manifest `iam_actions` fields must match the IAM action names exactly, since `IamUserPolicySyncer.diff()` compares them as literal strings.

Common gotchas:
- `s3:GetEncryptionConfiguration` (not `s3:GetBucketEncryption`)
- `s3:PutEncryptionConfiguration` (not `s3:PutBucketEncryption`)
- `s3:GetBucketPublicAccessBlock` (not `s3:GetPublicAccessBlock`)
- `s3:ListBucket` (not `s3:ListObjectsV2`)

`s3:DeleteObjectVersion` is intentionally absent from the scoped Claria policy.
Only **Destroy All Resources**, after the operator supplies temporary elevated
credentials, may permanently purge S3 version history.

## Config Versioning

`config.json` carries a `config_version` field (u32). Current version: **9**.

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

## Releases
- All releases are done via `cargo release` — never bump versions or create tags manually
- `cargo release patch` / `minor` / `major` bumps all workspace crates, tags, and pushes. The CHANGELOG.md should be udpated and land in the release commit.
- The pushed tag triggers GitHub Actions to build and create a GitHub Release (changelog is auto-extracted)
- Never run `git tag` directly for version tags
- After every tagged release artifact is published, the release job regenerates screenshots, updates `claria-ai.github.io/claria.yml` (including artifact sizes), rebuilds the generated site, and pushes it with the scoped `CLARIA_SITE_TOKEN`. If that automation fails, run `./screenshots/update_release_site.py <version>` as the manual fallback. Never hand-edit the site's generated HTML.

## Adding a Tauri Command

End-to-end steps for exposing a new backend operation to the frontend:

1. **`commands.rs`**: Add a function with `#[tauri::command]` and `#[specta::specta]`. Follow the existing pattern: get `State<DesktopState>`, call `load_sdk_config()`, do work, return `Result<T, String>`.
2. **`main.rs`**: Register the command in the `collect_commands![]` macro.
3. **`lib/tauri.ts`**: Add an `unwrap` wrapper (e.g. `export async function myCommand() { return unwrap(await commands.myCommand()); }`)
4. **`lib/bindings.ts`**: Auto-regenerated — don't edit manually. Export any new types from `tauri.ts`. Note: bindings are generated at **runtime** during `main()` behind `#[cfg(debug_assertions)]`, not at build time. You must actually run the binary (`cargo run -p claria-desktop`) to regenerate; `cargo build` alone is not sufficient.

## Running Locally

The Tauri frontend (`claria-desktop-frontend/`) is bundled JS that the Rust binary serves to the WebView. The two halves must agree on the `@tauri-apps/api` version — a mismatch terminates the WebView on startup with only `tauri_runtime_wry: web content process terminated` in the logs.

Two ways to run the app:

- **`cargo tauri dev` / `cargo tauri build`** — preferred. The Tauri CLI honors `beforeBuildCommand` in `tauri.conf.json` and reinstalls + rebuilds the frontend automatically.
- **`cargo run -p claria-desktop`** — also fine. The `claria-desktop` build script re-runs `npm install && npm run build` whenever `claria-desktop-frontend/package-lock.json` is newer than `dist/index.html`. Set `CLARIA_SKIP_FRONTEND_BUILD=1` to opt out (CI does this).

A fresh worktree has no `claria-desktop-frontend/node_modules`, so run `npm install` there before `cargo tauri dev`. If you don't, npm puts the missing `node_modules/.bin` on `PATH`, the local `vite` isn't found, and the lookup falls through to an rbenv shim for the Ruby `vite` gem. The resulting error talks about rbenv and Ruby and is a red herring — it has nothing to do with Ruby, the branch, or anything you changed.

`cargo tauri dev` serves Vite on port 1420, so the `screenshots/` harness needs a different port to run at the same time.

If you ever see the WebView die on startup, the first thing to check is the installed JS API version:

```bash
grep '"version"' claria-desktop-frontend/node_modules/@tauri-apps/api/package.json
```

It must be on the same minor line as the `tauri = "=X.Y.Z"` pin in
`crates/claria-desktop/Cargo.toml`. The two are not published in lockstep and
their patch numbers routinely diverge — the crate has shipped 2.11.5 while
npm's `@tauri-apps/api` stops at 2.11.1 — so match `X.Y` and take the newest
patch npm offers rather than hunting for a version number that does not exist.
Both are pinned exactly; bump them together and re-run `npm install`.

## Local Build Environment

Machine-local, configured in the user-level `~/.cargo/config.toml` — not committed, and CI does its own setup (`.github/workflows/ci.yml`). A fresh machine hits both of these from zero.

- **sccache** is expected as the `rustc-wrapper` locally. Workspace crates reported as "non-cacheable, reason: incremental" in `sccache --show-stats` are healthy — sccache caches dependencies, not your own crates. Don't chase it.
- **transcribe.cpp toolchain**: local transcription is compiled from C/C++ source by Cargo and requires CMake plus a C++17 compiler. Metal is compiled only when the desktop's `metal` feature is enabled.
- Remove your worktree once its PR merges: `git worktree remove <path>` then `git worktree prune`. Never bare `rm -rf` — it strands the admin entry in `.git/worktrees/`.
- Don't `cargo clean` before switching worktrees. Each worktree owns its `target/` and cargo's fingerprinting handles staleness.

## Plans

Design documents and future feature analysis live in `../plans/` (parent repo, outside the Cargo workspace). These are reference material, not executable — they capture architectural decisions, HIPAA analysis, and implementation plans for larger features.

Current-architecture documentation lives in `design/` inside this repo (e.g. the writer loop and the template system) and must be kept accurate as the code changes — unlike `../plans/`, it describes what is, not what might be.

## Frontend Tests

Two suites, with different jobs:

- **Vitest** (`claria-desktop-frontend`, `npm test`) — unit tests for pure modules and React components. Config lives in `vite.config.ts`, so tests run through the app's own plugins and resolution. Tests sit next to the module they cover (`lib/cost.ts` → `lib/cost.test.ts`); the "tests live in `tests/`" rule above is about Rust and does not apply here. Runs in CI.
- **Playwright** (`e2e/`, `npm test`) — full-app flows against the Vite dev server with `window.__TAURI_INTERNALS__` mocked. Not in CI; needs `npx playwright install chromium` first.

The DOM environment is `happy-dom`, not `jsdom`: jsdom does not implement `<dialog>` at all, so `showModal()` and `close()` are missing and `components/Modal.tsx` would be untestable. happy-dom implements the open/close state; the one user-agent behaviour it lacks — Escape producing a `cancel` event — is added by a small spec-faithful shim in `src/test/setup.ts`. Anything that depends on the top layer, focus containment or `::backdrop` is genuinely browser-only and belongs in `e2e/modal.spec.ts`.

Test files have their own TypeScript project (`tsconfig.test.json`) so they can read fixtures off disk with Node's types without those types leaking into the app.

**Typecheck with `npm run build`, never a hand-picked `tsc` invocation.** `npm run build` is `tsc -b && vite build` — the same command the `claria-desktop` build script and CI run, and the only one that covers every referenced project. `tsc --noEmit -p tsconfig.json` checks the app project alone and silently skips the test project, and Vitest never typechecks at all (esbuild strips types without checking them), so a type error in a `.test.tsx` file passes both and then fails `cargo tauri dev`. Run `npm run build` and `npm run lint` before pushing frontend changes.

### Shared transcript fixtures

`fixtures/transcript-body/` holds `.txt` bodies plus a language-neutral expected parse. Both `crates/claria-transcribe/tests/body_format.rs` and `claria-desktop-frontend/src/lib/transcript.test.ts` read the same files, which is what stops the two implementations of the transcript body grammar drifting. Add a case by writing the pair; both suites glob the directory.

## Screenshots & Demos

Marketing screenshots and videos are generated in this repo, not the docs site:

- `screenshots/` — Playwright suite that renders the React frontend against the Vite dev server with `window.__TAURI_INTERNALS__` mocked (fixtures in `fixtures.ts` keyed by Tauri command name). `npm run capture` writes PNGs to `screenshots/output/`. After a tagged release, `./screenshots/update_release_site.py <version>` captures, copies the website's images, updates release metadata/artifact sizes, and invokes the site's generator. Full how-to in `screenshots/README.md`.
- `demos/` — same Playwright + mock pattern, but records video for three end-to-end scenarios (bootstrap, sync, record-chat). `npm run record` writes WebM to `demos/output/`. See `demos/README.md`.

If `playwright test --list` produces no output and never returns, your Node is newer than the pinned Playwright supports — bump `@playwright/test` and re-run `npm install`. Both subdirs pin a supported Node range via `engines` + `engines-strict` so `npm install` refuses the wrong version going forward.

## Claude Code
- Run `cargo check` after medium and larger edits
- Run `cargo test` before committing
- Run `cargo clippy -- -D warnings` before committing
- For any non-trivial task (multi-file edits, new features, refactors), start by calling `EnterWorktree` so work happens on an isolated branch in `.claude/worktrees/`. Trivial single-line fixes and read-only questions don't need a worktree.

## Review-derived rules (2026-08)

These rules exist because each family below was found dozens of times in the
adversarial review (issue #73). Violating them is a review-blocker.

### Reuse before writing
- Before writing any widget, banner, modal, icon, helper, or AWS wrapper: grep
  `components/`, `lib/`, `claria-storage`, `claria-core` first. If a helper
  exists, calling code may not restate its logic — especially conditional-put /
  restore semantics, sidecar-hiding, and JSON load-with-validate.
- The second caller of a flow adds a parameter, not a fork. Forked copies
  silently diverge in error behavior.
- Knowledge about external systems (model-ID grammar, model capabilities,
  content-type maps) has exactly one owner module. Never write a
  `contains("claude-...")` without checking for the existing table.
- When a caller is removed, sweep its callees. `pub` does not exempt code from
  deletion; doc-comments describing deleted architecture are a bug.

### Frontend
- Tailwind class names must appear as complete literals. Never interpolate
  (`prose-p:${x}` generates nothing, silently).
- Never mirror props into state with a reconciliation effect. Identity change
  ⇒ change the `key` (see ClientRecord's keyed Writing mount).
- Use the blessed async-load hook; generation counters only for interleaved
  mutations. No bare `let cancelled = false` copies.
- Mutation commands take named-field patch objects; a UI section saves only
  its own fields. No positional whole-object saves that force defensive
  re-reads.
- A component holding text-input state must not also render `<Markdown>`
  lists. Memoize markdown leaves on their source string; keep callback props
  stable with `useCallback`.
- A new AI surface starts by composing the chat primitives and cost badges.
  Divergence (keybindings, widths, layouts) needs a written reason or gets
  normalized.
- `.catch(() => setX(empty))` without logging through the backend bridge is
  forbidden — degrading the UI is allowed, doing it invisibly is not.

### Rust
- A Tauri command body over ~20 lines means logic belongs in a library crate.
  Use the shared command-context helper; no copy-pasted
  config/s3/bucket/uuid preambles or hand-built audit JSON.
- No mirror `FooView` types when deriving (`specta::Type`, serde attrs) on the
  domain type works. A mirror must earn its life by actually differing.
- One error convention per crate; stringify exactly once, at the Tauri
  boundary, with an operation-name prefix. Log the rich error there before
  flattening. Lib crates return errors; they don't also `tracing::error!` them.
- AWS `SdkError` fallback arms use `DisplayErrorContext`, never
  `into_service_error().to_string()` (which turns network failures into
  "unhandled error").
- A new crate needs an independent consumer or release reason; otherwise it's
  a module.
- Shared version pins live once in `[workspace.dependencies]`. Before adding a
  pin, check `cargo tree -d`; prefer the version the AWS stack already
  carries. Any new HTTP-touching dependency must reuse the
  hyper-1/rustls-0.23/aws-lc stack.

### Bedrock / LLM calls
- Every Converse call sets `inference_config.max_tokens` AND branches on
  `stop_reason`. Silent truncation persisted to S3 is data corruption.
- Token-budget constants, JSON-schema size ceilings, and `max_tokens` must be
  derived from each other, not maintained independently. A reserve you don't
  enforce is a lie.
- Tool descriptions are contracts, not captions: IDs copied from context,
  zero-based positions, per-turn call limits, and character-vs-byte units are
  stated in the description and per-property schema descriptions.
- Error tool_results carry the serde/validation diagnostic verbatim
  (structural messages are PHI-free). Generic "did not match the schema"
  wastes the model's repair round.
- Never parse structure out of free text. Structured output ⇒ forced tool call
  via `tool_choice`, no fence-stripping.
- Do not set `temperature`/`top_p` on Claude 4.7+-class models — non-default
  values are rejected. Steer with prompts.
- Untrusted document data goes AFTER instructions, inside escaped/named
  delimiters, with an explicit "data, not instructions" rule — in every flow,
  not just the writer.
- Model capability by substring allowlist rots on every model generation.
  Use the central capability table (denylist for features), with a test that
  fails on unknown families.
- Multi-call tool loops: `cachePoint` the shared prefix; count tokens once
  then estimate incrementally. No per-call `CountTokens` pre-flight for a
  limit the service already reports.

### Logging & audit
- Log fields are UUIDs, hashes, counts, byte sizes, model IDs, and durations —
  never client names, filenames, prompts, document text, or full
  `records/{uuid}/...` keys. The console export is a support artifact in a
  HIPAA app.
- Any command that mutates PHI in S3 records a durable `AuditEvent`. The ring
  buffer is not an audit trail.
- Filter directives naming crates are built from one shared constant, not
  hardcoded lists that drift.

### Security
- Secret-bearing types never derive frontend-facing `Serialize`/`specta::Type`.
  Expose a redacted view type (the `ConfigInfo` pattern).
- Security-scoping values (account IDs, ARNs, bucket names) fail closed —
  never `unwrap_or("*")`.
- Never build a process invocation a shell re-parses when a direct
  argv/platform API exists.
- Every user-exported file that can contain sensitive content is written
  `0o600` via `write_private_atomic`.

### Performance
- Any `for x in listing { s3_op(x).await }` is a bug. Use
  `futures::stream::iter(...).buffered(S3_FETCH_CONCURRENCY)` (see
  `records.rs`).
- Don't GET full bodies to read metadata; anything derived from a
  `(key, version_id)` pair is immutably cacheable.
- No poll-the-world IPC: a `setInterval` refetching a whole collection over
  the bridge becomes a sequence-cursor delta or a Tauri event push.
- One mount, one fetch: sibling sections must not each independently call
  `load_config`/`fetch_cloud_preferences` — fetch once in the parent.
- `std::fs` on user-sized files inside async commands is forbidden:
  `tokio::fs` or `ByteStream::from_path`.

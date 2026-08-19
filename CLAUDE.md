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
- Workspace objects and their optimistic-concurrency protocol, immutable revisions, resumable drafting runs, attempt and per-call usage receipts
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

**`claria-docx-cli` — Template import diagnostics**
- `claria-docx` binary: reports how the importer classified every paragraph of a `.docx`, which style rule decided each heading, and what the appearance fallback would add
- A support tool, not part of the app — its own crate so `clap` and `eyre` stay out of the desktop binary
- Runs the real importer rather than a second reader, so its explanation cannot drift from the behaviour it explains

**`claria-mock-aws` — Fake AWS for tests**
- axum server that speaks the S3, STS, IAM, Bedrock, CloudTrail, Cost Explorer, Transcribe, and Artifact wire protocols
- Runs as a standalone binary or in-process via `testing::MockServer`, so tests drive real AWS SDK clients against an ephemeral port

### Boundary Rules
- Library crates accept `&aws_config::SdkConfig` — they never build their own SDK configs
- Library crates return `Result<T, CrateError>` — the caller decides how to present errors
- Library crates never do I/O to the local filesystem. Blessed exception: the provisioner's dual-write state persistence keeps a local safety-net copy — but it receives the state directory from the desktop caller (`build_persistence` parameter) and never derives a path itself
- `claria-desktop` is the only crate that reads/writes local config files (and the only crate that knows where local app directories live)
- Crates communicate through well-defined public APIs, not shared mutable state

## Code guide

Where the machinery actually lives, so a session goes straight to the file
instead of re-deriving the map. File paths and symbol names only — line numbers
rot.

### Writer draft-run trace

The money path, front to back. Each hop is one file.

| Hop | Where |
|---|---|
| Writing UI | `pages/Writing.tsx`, composing `components/DraftPlanPanel.tsx` (→ `DraftPlanCard.tsx`), `WritingCanvas.tsx`, `AgentThrobber.tsx` |
| State owner | `lib/useReportWorkspace.ts` — owns `agentActivity` and the `DraftRunUiState`, and is the only place the progress channel is consumed |
| Reducer | `lib/draftRun.ts` — `DraftRunUiState`, `emptyDraftRun`, `runStateFromDraftRun`, `reduceDraftRun`, `overlaySections` |
| Bridge | `lib/tauri.ts` — `generateDraftPlan`, `updateDraftPlan`, `startDraftRun`, `resumeDraftRun`; each mints the `Channel` |
| Command | `claria-desktop/src/commands/plan.rs` — the same four commands, each opening a `StopRegistration` (`commands/streams.rs`) keyed by `stream_id` |
| Plan pass | `claria-report-pipeline/src/plan.rs` — `generate_draft_plan` → `plan_fresh_run`, a sequential loop of `PLAN_BATCH_SECTIONS`-section batches over `AnalysisPass::run_call`; resume via `plan_draft_resume`, which calls the same core once |
| Plan system blocks | `plan.rs::analysis_system_blocks` — `prompts.rs::planner_system_prompt`, then the corpus block from `record_context.rs::load_full_record_context` (one compact JSON blob, per-file bound `claria_core::record_text::MAX_RECORD_TEXT_BYTES`), then `full_draft_context.rs::planner_template_context` (pretty-printed, structure only — the writer's `template_context` is what carries `template_body`) |
| The model call | `claria-bedrock/src/analysis.rs::converse_structured`, forcing `SUBMIT_SECTION_PLAN_TOOL` once per batch; decoded by `decode_section_plan`, checked by `plan.rs::validate_section_plan` against the batch's own section list |
| Approved plan → drafting | `parallel_draft.rs` fan-out, `buffer_unordered(BEDROCK_FAN_OUT_CONCURRENCY)` (3, in `lib.rs`); the sequential tool-loop shape lives in `turn.rs` |
| Completion gate | `gate.rs::evaluate_report_completion` |
| Run lifecycle | `run.rs` — `resume_draft_run`, `finalize_partial_draft`, `abandon_draft_run`, `park_stopped_run`, `release_failed_run` |

**Progress comes back over an IPC Channel, not Tauri events.** The pipeline emits
`ReportTurnProgress` (`claria-report-pipeline/src/turn.rs`); `claria-desktop`
mirrors it as `ReportTurnProgressView` with a `From` impl in
`src/report_authoring.rs`; the command pushes it down a
`tauri::ipc::Channel<ReportTurnProgressView>` supplied by the caller;
`lib/draftRun.ts` reduces it. Chat streams the same way
(`ChatStreamEvent` in `commands/chat.rs`). Nothing in either flow uses
`app.emit`. Adding a variant means all four edits plus regenerated bindings —
and `reduceDraftRun`'s `default:` arm silently swallows kinds it doesn't know.

Durable state for a run lives under `report-authoring/{client}/`, minted in
`claria-core/src/s3_keys.rs`: the session workspace at `sessions/{report_id}.json`
(`workspace.json` is the legacy singleton), the run at
`runs/{report_id}/{run_id}.json`, review findings at `findings/{report_id}.json`.
The run object is rewritten after every section that lands — that is what makes
a run resumable.

### Bedrock plumbing map (`crates/claria-bedrock`)

| File | Owns |
|---|---|
| `converse.rs` | Stream bounds, cache points, budgets, usage/budget logging, `StopSignal` |
| `retry.rs` | `with_throttle_retry` |
| `analysis.rs` | Forced-tool structured calls: `StructuredCallRequest`, `converse_structured`, `AnalysisInputBudget`, the tool schemas |
| `report.rs` | Writer turns and `DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE` |
| `chat.rs` | Chat turns and `CHAT_MAX_OUTPUT_TOKENS` |

**Stream silence** is bounded per call by a `StreamBounds` pair in
`converse.rs`: a first-frame wait (in `start_converse_stream`) and an idle wait
(per-`recv` in `recv_stream_event`, clock restarting each frame). These exist
because the AWS SDK's stalled-stream protection does not cover `ConverseStream`
— the generated operation registers no `StalledStreamProtectionInterceptor` —
and the SDK read timeout bounds only the wait for response headers.

Both are clinician-configurable through `BedrockRuntimeLimits`, which reaches
the writer as `ReportTurnLimits::stream_bounds` and the planner/reviewer through
`DraftPlanRequest::with_stream_bounds` / `ReviewSweepRequest::with_stream_bounds`.
The family defaults are unchanged (`DEFAULT_STREAM_*` 90s/60s conversational,
`DEFAULT_ANALYSIS_STREAM_*` 120s/90s), and every configured value is clamped to
`1..=MAX_STREAM_TIMEOUT_SECS` in `StreamBounds::writer`. Chat is not
configurable and still reads the const default.

**The writer's output ceiling** is configurable the same way. The `max_tokens`
one writer call sends and the reserve subtracted from the model's window are the
same number by construction — `ReportInputBudget` carries it, and
`converse_report_with_tool_set` reads it back off the budget rather than a
const. `effective_output_reserve` clamps a configured ceiling to
`MIN`/`MAX_REPORT_OUTPUT_TOKEN_RESERVE` and then to half the model's window, so
raising it can never leave a budget of nothing. The planner and reviewer
reserves stay compile-time: both are tied to their JSON-schema ceilings by
`const` assertions.

**Budgets** are the model's context window minus a per-operation output reserve.
The window comes from the central capability table
`claria-core/src/model_id.rs::ModelCapabilities::for_id`, which is suffix-driven
(`:48k` / `:200k` / `:1m`) and otherwise an assumption. Reserves:
`PLAN_OUTPUT_TOKEN_RESERVE` and `REVIEW_OUTPUT_TOKEN_RESERVE` in
`claria-report-pipeline` (`plan.rs`, `review.rs`), the writer's configured
reserve (defaulting to `DEFAULT_REPORT_OUTPUT_TOKEN_RESERVE`)
and `CHAT_MAX_OUTPUT_TOKENS` in `claria-bedrock`. Actual counting is
`converse.rs::InputTokenBudget` — `exact` counts once then estimates at ~4
chars/token, `estimated` trusts the estimate until within 10% of the budget, and
`seeded` starts a fan-out sibling from a count a warm branch already paid for.

**Retries.** `retry.rs::with_throttle_retry(label, op)`: 4 attempts total,
jittered 1s/2s/4s, retrying only `is_retryable_throttle` or
`is_interrupted_before_completion`. Schema violations, truncated responses, and
exhausted quotas return on the first attempt. It notifies through
`tracing::warn!` and, via `with_throttle_retry_observed`, an optional
`RetryObserver` — which is how the writer surfaces `ModelCallRetrying`.

**Forced-tool calls** share one tool configuration —
`analysis.rs::analysis_tool_configuration` returns all three tools
(`submit_section_plan`, `submit_resume_plan`, `submit_review_rows`) with no
`toolChoice`, and `converse_structured` stamps the per-call `toolChoice` on top.
A differing tool list would move the tools cache tier and cost every role the
corpus prefix the others paid to write. `StructuredCallRequest.on_partial_tool_input`
is a display-only callback over the partial tool-input JSON (the planner's row
counter uses it); nothing is parsed out of the partial buffer.

**Operation labels** — `"report_plan"`, `"report_review"`, `"report"`,
`"report_parallel_draft"`, `"chat"` — are per-call-site literals, not a shared
enum. They are what `log_model_budget`, `log_turn_usage`, and the retry WARN key
off, so a mislabelled call site is invisible in a console export.

### Chat trace

`commands/chat.rs::chat_message` → `claria-bedrock/src/chat.rs`:
`chat_input_token_budget` (window − `CHAT_MAX_OUTPUT_TOKENS`) → `log_model_budget`
→ `InputTokenBudget::estimated` → `chat_converse_stream`, which streams deltas
back over `Channel<ChatStreamEvent>`. History objects live at
`records/{uuid}/chat-history/{chat_id}.json`
(`claria-core/src/s3_keys.rs::chat_history`).

### Logging and the Console

- **Ring buffer.** `claria-desktop/src/console.rs` — `ConsoleLayer` is a tracing
  layer writing into a `ConsoleBuffer` capped at `MAX_BYTES` (10MB). Span-close
  entries carry `elapsed_ms`.
- **Polling.** `pages/Console.tsx` polls `get_console_logs_since(seq)` every
  `POLL_INTERVAL_MS` (500ms) and applies the `ConsoleDelta`.
- **On-disk logs.** `claria-desktop/src/logging.rs` — daily rolling files,
  `MAX_LOG_FILES` kept, under `app_log_dir()` (macOS:
  `~/Library/Logs/com.claria.desktop`). The crate list every filter is built
  from is the single `CLARIA_CRATES` constant; never hardcode a second list.
- **Frontend bridge.** `lib/logBridge.ts` → `commands/console.rs::log_frontend_event`,
  logged under target `claria_desktop::frontend`, capped at 2000 chars and
  stripped of control characters.
- **Targets that look like modules but aren't.** `claria_bedrock::budget` and
  `claria_bedrock::cache` are `target:` overrides on individual `tracing::info!`
  calls in `converse.rs`, not modules. Grepping for a module by that name finds
  nothing.
- PHI rules for what may appear in a log field are in **Logging & audit** below.
  The console export is a support artifact in a HIPAA app.

### Gotchas

- The writer flow's progress transport is an IPC `Channel`, not Tauri events.
- `lib/bindings.ts` regenerates only when the binary actually **runs**
  (`#[cfg(debug_assertions)]` in `main()`); `cargo build` is not enough.
- The `log_model_budget` INFO line is the **allowance** (window − reserve), not
  the measured size of the request. The `CountTokens` result is a separate thing.
- A retry is only visible to the reader if the call site passes a
  `RetryObserver`; the plain `with_throttle_retry` still logs and nothing more.
- One `AnalysisInputBudget` and one `PlanRowCounter` live on `AnalysisPass`,
  outside the batch loop. Building either inside it silently buys a
  `CountTokens` per batch and restarts the reader's row count at every
  checkpoint.
- `PLAN_BATCH_SECTIONS` and `PLAN_OUTPUT_TOKEN_RESERVE` are checked against
  each other by a `const` assertion in `plan.rs` deriving
  `WORST_CASE_PLAN_ROW_CHARS` from the schema's own ceilings. Widening the
  evidence schema without widening the reserve fails the build.
- A new `ReportTemplateWarningCode` variant that is not added to
  `WARNING_ORDER` in `import.rs` is counted and then silently discarded —
  `into_sorted` uses that list as a filter, not just an ordering.
- `claria-desktop` builds its own Tokio runtime (see **Async runtime**). Any
  `spawn` or `block_on` before `async_runtime::install()` initializes the
  global one and makes the install panic.

### Feature map

The surfaces, and what owns each. Ownership is stated only where it was
traced; see **Coverage of this guide** for what is listed but unexplored.

| Surface | Frontend | Backend |
|---|---|---|
| Writer (whole-report drafting) | `pages/Writing.tsx`, `lib/useReportWorkspace.ts`, `lib/draftRun.ts`, `lib/draftPlan.ts` | `claria-report-pipeline` (`plan.rs`, `turn.rs`, `parallel_draft.rs`, `run.rs`, `gate.rs`), `claria-report-store`, `claria-bedrock/report.rs` |
| Review / findings | `lib/findings.ts` | `claria-report-pipeline/review.rs`, `claria-bedrock/analysis.rs` |
| Chat (per-client and infra) | `pages/ClientChat.tsx`, `pages/InfraChat.tsx` | `commands/chat.rs`, `claria-bedrock/chat.rs` |
| Preferences | `pages/Preferences.tsx`, `lib/preferencesNav.ts`, `lib/preferencesSearchContent.ts` | `claria-desktop/config.rs`, `commands/config.rs` |
| Templates (import/export) | template panes inside Writing | `claria-docx`, `claria-docx-cli`, `report_template_commands.rs` |
| Client records | `pages/ClientList.tsx`, `ClientRecord.tsx`, `RecordTab.tsx`, `ClientRecordSettings.tsx` | `claria-records`, `claria-storage` |
| Transcription | `lib/transcribe.ts`, `lib/useMemoRecorder.ts`, `lib/transcript.ts` | `claria-transcribe` |
| Cost / billing | `pages/CostExplorer.tsx`, `lib/cost*.ts`, `lib/usePricingMap.ts` | `claria-billing` |
| Provisioning / onboarding | `pages/Provision.tsx`, `StartScreen.tsx`, the `*Guide.tsx` pages | `claria-provisioner`, `commands/provision.rs` |
| Console / diagnostics | `pages/Console.tsx`, `lib/logBridge.ts` | `console.rs`, `logging.rs` |

Chat and the Writer are separate flows that happen to share `claria-bedrock`.
They do **not** share prompt composition, retry policy, stream bounds, or
budget code, and a change to one is not a change to the other. The writer is
the only flow with a tool loop, a plan gate, durable runs, or per-call usage
receipts.

### Preferences trace

Two stores, deliberately: `config.json` on disk (machine-local, versioned by
`CURRENT_VERSION`) and `_state/preferences.json` in S3 (synced across a
clinician's machines, versioned independently by `PREFERENCES_VERSION`).
`SyncedPreferences` is the subset that travels.

Reads flow `useSyncedPreferences()` → `load_config` → `ClariaConfig`, and the
redacted `ConfigInfo` is what reaches the frontend — secret-bearing fields
never derive `Serialize`/`specta::Type`.

Writes are **patch-saves**: `savePreferencesPatch({ report_authoring: draft })`
sends only that section's fields, so one pane cannot roll back another's edit.

`load_config_at` calls `report_authoring.validate()` and **fails the load** on
an out-of-range value. Bad settings are refused at startup, not at the call
site that would have used them.

Adding one writer-limit field touches, in order:

1. `claria-desktop/src/config.rs` — field, `#[serde(default = "…")]`, the
   default fn, and a migration block (bump `CURRENT_VERSION`)
2. `claria-report-pipeline/src/lib.rs` — `DEFAULT_*`/`MAX_CONFIGURABLE_*`
   consts, validation, accessor
3. `pages/Preferences.tsx` — `WRITER_LIMIT_DEFAULTS`, the field-descriptor
   array, `normalizeWriterPreferences`. `WriterLimits` is
   `Required<ReportAuthoringPreferences>`, so the type follows the bindings
4. `lib/preferencesNav.ts` — an anchor entry, or settings search cannot find it
5. `lib/bindings.ts` — regenerated by **running** the debug binary
6. `e2e/tauri-mock.ts` — every `report_authoring` fixture, or the Writing tab
   drops to its error boundary (this is what broke release screenshots once)
7. tests: `claria-report-pipeline/tests/limits.rs`,
   `claria-desktop/tests/config_load.rs`

Field-label constants in `claria-report-pipeline/src/lib.rs`
(`TOOL_ROUNDS_FIELD_LABEL`, `IDLE_TIMEOUT_FIELD_LABEL`, …) are quoted verbatim
in failure messages and must match the labels in `Preferences.tsx`, or an
error sends a clinician looking for a control that does not exist.

### Two writer executors, two retry budgets

`turn.rs` and `parallel_draft.rs` are both executors for `FullReportRequest`,
and they do not retry alike:

| | sequential (`turn.rs`) | parallel (`parallel_draft.rs`) |
|---|---|---|
| Retries | `STREAM_INTERRUPTION_RETRIES = 2` → **3 attempts** | `retry::MAX_ATTEMPTS = 4`, jittered 1s/2s/4s |
| Backoff | fixed `STREAM_INTERRUPTION_RETRY_DELAY` | `backon` exponential |
| Concurrency | one call at a time | `BEDROCK_FAN_OUT_CONCURRENCY` (3) branches |

"N attempts" in a console log therefore tells you which executor ran. Both end
at `map_bedrock_failure`, so failure prose is shared even though the policies
are not.

Section count drives everything downstream: a plan of N sections becomes N
branches, and `ReportTurnLimits::scaled_for_plan` raises the call and round
ceilings to fit. One section means one branch writing the entire report in a
single response — which is how a template with no heading styles turns into a
stream that goes quiet past its idle bound.

### Frontend shape

Vite + React + Tailwind, no router library — `App.tsx` switches on state.
Pages in `src/pages/`, everything reusable in `src/lib/` (hooks and pure
modules side by side, tests next to their module).

- **Rust is the only source of types.** `lib/bindings.ts` is generated;
  `lib/tauri.ts` wraps each command in an `unwrap` that turns
  `Result<T, String>` into a throw.
- **Progress arrives on an IPC `Channel`**, minted per call in `lib/tauri.ts`,
  never Tauri events. `lib/draftRun.ts` is a pure reducer over those events —
  which is why it is unit-testable and why its `default:` arm silently
  swallows event kinds nobody added.
- `lib/useAsyncLoad.ts` is the blessed load hook; the review rules forbid
  hand-rolled `let cancelled = false`.
- Preferences panes are `NavPane`s with `data-pref-anchor` attributes, which
  is what settings search scrolls to.

### Testing topology

~109 Rust test binaries; frontend is Vitest (46 files) plus a non-CI Playwright
suite in `e2e/`.

`claria-mock-aws` speaks the real wire protocols, so tests drive genuine AWS
SDK clients at an ephemeral port. Its failure knobs matter more than its happy
path — `state.rs` exposes `bedrock_stream_stalls` (starts, then goes quiet),
`bedrock_stream_silences` (never sends a first frame), `bedrock_stream_drops`
(severs mid-response), `bedrock_stream_stalls_after`, and
`ScriptedBedrockResponse` queues.

To test a timeout without waiting it out, spawn a task that sleeps briefly in
real time and then calls `tokio::time::pause()`; virtual time then jumps the
bound instantly. `converse_stream.rs` does this, and it needs tokio's
`test-util` feature.

That trick has a limit worth knowing: at the **pipeline** layer it races the
AWS SDK's own dispatch-retry, which under parallel test load surfaces a
`DispatchFailure` before our bound fires. Timeout *behaviour* is pinned at the
`claria-bedrock` layer; the pipeline's *reaction* to it is pinned by calling
`interruption_advice` directly (`tests/failure_advice.rs`). Reproducing both
through a live stream is not worth the flake.

### Release mechanics

`cargo release minor --execute --no-confirm` from a clean `main`. The
`pre-release-replacements` live in **`crates/claria-desktop/Cargo.toml`**, not
the workspace root — they rewrite `tauri.conf.json` and the CHANGELOG's
`## [Unreleased]` header. `shared-version = true` moves every crate together,
so a **new crate must be created with the current workspace version** or the
release warns and bumps it out of step.

Wait for `main`'s CI before tagging: the tag starts the artifact build, and CI
runs ~13 min with the Release job ~18 min (Windows tests are the long pole).
Merging a second PR cancels the first's in-flight run — a `cancelled` status on
the older merge is normal, not a failure.

### Coverage of this guide

Written from work actually done in the repo, so it is uneven on purpose.

**Traced end to end:** the writer draft run and both its executors; Bedrock
call plumbing (bounds, budgets, retries, cache points); the preferences chain
from Word-visible setting to Bedrock request; DOCX import and section carving;
the async runtime and its stack requirements; the release process.

**Listed but not explored** — treat the table above as a map of *where* to
look, not a claim about how they work: transcription, provisioning and IAM
setup, cost/billing, the records and storage internals, the review/findings
model beyond its call shape, DOCX *export* (`render.rs`,
`template_render.rs`), `claria-eval`, and most page components other than
`Preferences.tsx`.

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
| `report-authoring/{uuid}/runs/{report_id}/{run_id}.json` | Durable per-section state for one resumable drafting run |
| `report-authoring/{uuid}/findings/{report_uuid}.json` | Review findings for one Writing session, with apply/undo history |
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

`config.json` carries a `config_version` field (u32). Current version: **13**.

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

## Async runtime

`claria-desktop` builds the Tokio runtime itself
(`async_runtime::install`) instead of letting Tauri create one lazily, and
gives each worker an 8 MiB stack. Tauri's default is Tokio's, which is the
2 MiB a platform thread gets, and that is not enough for one AWS SDK request
in an unoptimized build: the smithy orchestrator, hyper pool, TLS connector
and rustls handshake nest as futures rather than calls, so a handshake sits
about a hundred `poll` frames deep before webpki starts parsing DER. Release
builds survive only because `opt-level = "z"` and LTO inline the combinators
away; `cargo tauri dev` overflows the guard page and aborts.

`install` must run before anything spawns — `tauri::async_runtime::set`
panics once the global runtime exists, and the first `spawn` or `block_on`
anywhere creates it.

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

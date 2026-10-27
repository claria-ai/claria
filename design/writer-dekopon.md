# Writer harness checkpoint

This is a local Writer harness integration with synthetic validation, not shipping approval.
The starting Claria `main` was `bdd3731491b71f3ef87a530c19d220a324567230`.
The inspected Dekopon `main` was `0d4f4276071cf41fa36808eea0cd9b8d2d13bf5f`.
The first embedded-package checkpoint is `94acc810f6e96cb6f48ec10065fdab5f0c8a788d`.
The current paired embedded commit is `c752363b56eee6eb202302a3aabf574eb72980ab`.
The separate extraction-model/error-sidecar release is outside this work.

## Boundary

Claria calls Amazon Bedrock Runtime `ConverseStream` through the AWS Rust SDK.
`claria-desktop/src/aws.rs` owns credential selection and builds `SdkConfig`;
libraries receive that configuration. This work introduces no credential lookup,
service proxy, IAM change, AWS resource, or live-model test.

The proposed dependency is the Claria fork's isolated `dekopon-embedded` package.
It adapts Dekopon's async inference, bounded turn loop, complete-message replay,
explicit tool authority and cancellation patterns to native Bedrock content.
SDK messages retain ordered reasoning/signatures, tool-result status, stop reasons,
cache blocks and optional raw token usage. The host supplies the allowed tools and
executes them. The model never receives an S3 client or authority to save a report.

Importing `dekopon-agent` directly was rejected because its existing loop is tied
to shell/meta tools and its dependency graph includes broker protocol, shell,
configuration, process and telemetry code. Its prompt/answer/tool payload tracing
also conflicts with Claria's privacy requirement. A model-only import still brings
unneeded authentication/HTTP stacks and incompatible dependency pins. Running a
gateway/broker service would add an unnecessary deployment boundary.

The separate package preserves upstream history and licenses and leaves the native
Dekopon workspace unchanged. Its privacy rules are deliberately scoped to Claria's
embedding: metadata-only diagnostics, no document text, prompts, identifiers,
tool arguments or credentials. Claria's S3 durability remains Claria's responsibility.

## Functionality contract

| Existing owner | Contract retained |
| --- | --- |
| `claria-core/models/report.rs`, `claria-report-store/workspace.rs` | Named sessions, accepted document, expected revision, ETag conditional writes, immutable revisions, restore and proposal accept/reject |
| `claria/src/context.rs`, `prompts.rs`, `prompt_cache.rs` | Current user edits/focus, full outline, bounded body reads, fixed trust rules after custom prompts, sanitized durable history and exact transient replay |
| `claria/src/tools.rs` | Mode-specific tool allowlist, strict typed arguments, bounded record/section reads, one proposal, section-scoped writes and durable section commits |
| `claria/src/plan.rs`, `parallel_draft.rs` | Editable plan and approval gate, three bounded section branches, curated evidence, warm cache/token-count seed and coordinator-owned merges |
| `claria/src/run.rs` | Interrupted-run resume, partial finalize, abandon, run ownership and completion save even when Stop races finalization |
| `claria/src/gate.rs`, `review.rs` | Citation filename/quote checks, review/fix/undo/dismiss and completion findings |
| `claria-bedrock` | Context/output budget coupling, CountTokens reuse/fallback, model capability/tuning, cache placement, raw usage pricing and retry advice |
| `useReportWorkspace.ts`, `draftRun.ts`, `Writing.tsx` | Existing IPC channel, activity/progress, manual edits, proposal diff, usage, templates and DOCX export; no UI changes |

The sequential Writer and planned section branches now implement the embedded
`Session` interface. One `drive` owns bounded model calls, inference invocations,
retry waits and cancellation; the native embedded tool-host adapter uses it too.
Claria retains native domain responses, protocol assembly and tool dispatch, so
there is no second conversation representation or parallel duplicate model loop.
Sequential retries remain three attempts with fixed two-second interruption waits;
parallel retries remain four attempts with the existing jitter and elapsed budget.

Every completed response records known usage before any tool effect or repair call.
Unknown usage remains unknown after an observed failed invocation, even when a retry
succeeds. Pre-call Stop and definite local preflight failures retain known zero.
Parallel receipt IDs use the existing unique call reservation; completed-call totals
remain a count, so receipt IDs can have gaps after interrupted calls.

Claria already accepted a complete response as a batch. The host explicitly defers
Stop while saving its receipt and executing that batch, then refuses another call.
The existing successful-finish cut still defers Stop through revision finalization.
A fatal parallel storage error cancels remaining inference and drains accounting,
including accepted receipt writes, without applying further document effects.
An inference operation includes its token-count preflight; cancellation can race an
already admitted operation, and completed responses are still receipted.

Resume continues to reload the existing run, workspace and evidence snapshot,
retaining landed sections and rejecting incompatible user revisions or proposals.
No status database, raw transcript persistence or new ownership model was added.
These mechanics do not demonstrate better real-model writing.

## Validation plan

Run the unchanged `claria-bedrock` and `claria` suites before integration, then run
the same suites through the fork backend. These use genuine AWS SDK clients with
local synthetic wire transports, never AWS endpoints. Inspect outbound tools,
cache/tuning/max-token fields, reasoning replay, tool-result correlation, usage
receipts, revisions, stop/retry and partial/resume behavior. Add focused coverage
for stopping while waiting for headers/first frame.

The backend fixtures additionally exercise fragmented text/tool/reasoning blocks,
malformed known events, bounds, absent usage, terminal-state errors and unknown
events. The generic tool-host scenarios cover evidence use, recoverable tool
failure, revision staging, bounded work and output fidelity.

No frontend files are changed in this slice. Any later UI change requires browser
screenshots and owner approval before a separate adversarial UI review.
Non-UI correctness review proceeds independently.

## Checkpoint limits

The GitHub integration denied creation of `claria-ai/dekopon` with HTTP 403
(`Resource not accessible by integration`) after repository lookup and an
organization-wide repository listing confirmed no existing fork. No fork was
created, and the denied operation has not been retried. The local dependency path
is temporary; a reproducible fork commit pin and paired draft PRs require the fork
to exist and be writable. No upstream changes, merge, release or deployment occur.

Live AWS/model access, real tool-generation quality, real Bedrock reasoning/cache
behavior, production latency/cost and native desktop behavior remain unvalidated.
The owner will provide test access later. Parent accepted the first protocol slice
and authorized this bounded-kernel migration; merge/release/deployment remain out
of scope.

The separate package's dependency audit currently fails: Claria's existing exact
Tokio 1.52.3 pin is affected by RUSTSEC-2026-0386, and the native Dekopon duplicate
crate policy rejects the AWS SDK's `http`/`http-body` generations. No exception or
uncoordinated dependency bump was added. The isolated passing option and exact
advisory sources are recorded in [dependency options](writer-dependency-options.md).
Adopting the coherent AWS cohort requires coordination with the active release.

The stopped-attempt and recovered-retry paths now retain unknown usage when an
in-flight request has no trustworthy receipt. Post-terminal failures remain
unretried. Provider billing after cancellation still requires live validation.

The shared SDK configuration still governs SDK-internal retries. The driver
counts inference invocations; the backend separately counts SDK signing attempts
with a per-request callback that leaves the request unchanged. A recovered SDK
retry preserves final-response tokens but marks both the call receipt and attempt
incomplete. This is conservative accounting, not proof of provider billing or a
count of every possible transport-level retry.

An inherited process-death gap remains: `load_resumable_draft_run` returns the
workspace's active run, but `resume_draft_run` accepts only `Stopped` or `Failed`.
A process killed while the run is `Drafting` can therefore leave a visible run
that cannot be resumed directly. Automatically reclaiming it could race another
window or machine. A takeover/lease decision is required; this slice does not
silently change that ownership contract.

## Observed evidence

On Linux with Rust 1.98.1:

- The unchanged baseline passed 264 tests; six existing live tests stayed ignored.
- The first transport checkpoint passed 268 tests with the same six ignored tests.
  Its four new cases cover exact
  streamed reasoning/tool replay, first-frame cancellation, a post-terminal broken
  trailer with durable unknown usage, and retry of a typed stream exception.
- The current `cargo test -p claria-bedrock -p claria --locked --no-fail-fast`
  passed 280 tests, with six existing live tests ignored. Receipt/tool-write gates,
  partial streams, interrupted retries, SDK retries, batch draining and reload/resume
  are exercised against loopback AWS fixtures. `/tmp/writer-final-tests.log` records
  the exact run; `CARGO_TARGET_DIR=/workspace/writer-target` and
  `CARGO_PROFILE_DEV_DEBUG=0` were used.
- `cargo clippy -p claria-bedrock -p claria --all-targets --locked -- -D warnings`
  passed on the final integration (`/tmp/writer-final-clippy.log`). A pre-existing late-initialization warning was fixed
  in the touched writer context initializer without changing its values.
- The separate embedded package now passes 33 tests (19 kernel/lifecycle,
  13 collector fixtures, one typed-exception unit test), all-target Clippy with
  warnings denied, nightly formatting and rustdoc with warnings denied.
- The `claria-bedrock` dependency tree contains only `dekopon-embedded` from Dekopon;
  it adds no gateway, broker, Wasmtime, shell, OpenTelemetry, reqwest or ureq.
- Independent non-UI review verified completion-phase usage, typed exceptions,
  cache replay, reused tool IDs, receipt/effect ordering, fatal sibling accounting
  drain and per-invocation SDK attempt counters. No blocking source finding remains.
- No frontend, layout, stylesheet, generated binding or desktop command changed.
  No browser/UI review was performed.

These checks do not cover the whole native desktop workspace, remote CI or a
released artifact. The inherited native Dekopon workspace was not rebuilt: its
source, manifest, lockfile and contracts are unchanged.

The new Claria fixtures are in `tests/stop.rs`, `tests/parallel_draft.rs`,
`tests/writer_backend_failure.rs` and `tests/writer_durability.rs` under
`crates/claria`. They check known/unknown accounting, exact Unicode content and
proposal effects, byte-identical retry inputs, durable receipts before repair,
accepted batch completion, no next call after Stop, and resume from stored sections.
The fatal-drain fixture directly covers an in-flight sibling repair and a receipt
failure; genuinely queued branches and a ready successful sibling after a fatal
error are source-reviewed, not separately fixture-tested. The dependency probe
uses the older 23-test checkpoint and is not a full-workspace upgrade validation.

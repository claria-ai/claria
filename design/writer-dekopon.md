# Writer harness checkpoint

This is a local Writer harness integration with synthetic validation, not shipping approval.
The starting Claria `main` was `bdd3731491b71f3ef87a530c19d220a324567230`.
The inspected Dekopon `main` was `0d4f4276071cf41fa36808eea0cd9b8d2d13bf5f`.
The first embedded-package checkpoint is `94acc810f6e96cb6f48ec10065fdab5f0c8a788d`.
The current paired embedded commit is `6dfae22e7c20a78a485c3e0e9e8f880777551625`.
The separate extraction-model/error-sidecar release is outside this work.

## Boundary

Claria calls Amazon Bedrock Runtime `ConverseStream` through the AWS Rust SDK.
`claria-desktop/src/aws.rs` owns credential selection and builds `SdkConfig`;
libraries receive that configuration. Explicit profile selection now uses the
SDK's profile provider directly, as approved by the owner; ambient credentials
cannot replace that selection. Default-chain and inline credential behavior stay
unchanged. This work introduces no service proxy, IAM change, AWS resource, or
live-model test.

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
| `useReportWorkspace.ts`, `draftRun.ts`, `Writing.tsx` | Existing IPC channel, activity/progress, manual edits, proposal diff, usage, templates and DOCX export; bounded history/findings keep resume controls reachable |

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

Failure cleanup now reloads durable sections before stamping failure, rather than
replaying an in-memory section PUT that storage refused. It captures the workspace
ETag before publishing `Failed`, and a conflicting run write or newer workspace
claim fences cleanup. Lost workspace-claim replies reconcile by exact durable
body; an uncommitted claim rolls back only through its own run ETag. Completion
healing requires this run's section authorship at its expected revision cut,
rather than treating an unrelated later edit as proof of completion. These repair
inherited persistence hazards exposed by crash/fault fixtures; they add no format
or database.

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

The only application UI change bounds the existing history/findings area while a
draft plan is open. Existing components, styles and controls are reused. Browser
fixtures use the real React page with mocked Tauri commands, not a native desktop.
Screenshots require owner approval before independent adversarial UI review;
non-UI correctness review proceeds independently.

## Checkpoint limits

The GitHub integration denied creation of `claria-ai/dekopon` with HTTP 403
(`Resource not accessible by integration`) after repository lookup and an
organization-wide repository listing confirmed no existing fork. After the owner
changed installation access on 2026-10-27, Claria permission checks succeeded;
one authorized retry of the fork endpoint still returned the same HTTP 403.
No fork was created, and that operation is stopped. The preserved Claria branch
is published as [draft PR 148](https://github.com/claria-ai/claria/pull/148), based
on the unchanged remote `main` recorded above. Its temporary sibling path prevents
clean-checkout Rust CI from building until the fork exists and an exact writable
fork commit can be pinned. Publication is not test or shipping approval. No
upstream changes, merge, release or deployment occur.

Live AWS/model access, real tool-generation quality, real Bedrock reasoning/cache
behavior, production latency/cost and native desktop behavior remain unvalidated.
The owner will provide test access later. Parent accepted the first protocol slice
and authorized this bounded-kernel migration; merge/release/deployment remain out
of scope.

The separate package now passes the unchanged native Dekopon dependency policy
with Tokio 1.53.3 and the coherent September AWS SDK cohort. Claria's matching
service pins and active transport fixes are isolated to this Writer branch.
Fifteen inherited diagnostics remain in a broader Claria advisory scan; no policy
exception was added. Exact versions, advisory sources and compatible options are
recorded in [dependency options](writer-dependency-options.md). Integrating this
branch still requires coordination with the separate release; that branch was
not modified.

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
window or machine. Actual future-abort fixtures verify that implicit resume and
partial finalization refuse that state without making inference calls. Explicit
abandon fences stale writes and permits a fresh run. Automatic same-run takeover
would require an explicit ownership decision and is not added. A lost first run
claim reply or unavailable claim readback can retain this guarded state. Erased
section authorship prevents automatic completion healing rather than inventing
proof; recovering an old partial-finalization flag also remains outside the
existing data format.

## Observed evidence

On Linux with Rust 1.98.1:

- The unchanged baseline passed 264 tests; six existing live tests stayed ignored.
- The first transport checkpoint passed 268 tests with the same six ignored tests.
  Its four new cases cover exact
  streamed reasoning/tool replay, first-frame cancellation, a post-terminal broken
  trailer with durable unknown usage, and retry of a typed stream exception.
- The bounded-kernel checkpoint passed 280 Claria/Bedrock tests, with six existing
  live tests ignored. Receipt/tool-write gates,
  partial streams, interrupted retries, SDK retries, batch draining and reload/resume
  are exercised against loopback AWS fixtures. `/tmp/writer-final-tests.log` records
  the exact run; `CARGO_TARGET_DIR=/workspace/writer-target` and
  `CARGO_PROFILE_DEV_DEBUG=0` were used.
- All-target Clippy with warnings denied passed on the kernel checkpoint
  (`/tmp/writer-final-clippy.log`). A pre-existing late-initialization warning was fixed
  in the touched writer context initializer without changing its values.
- The dependency-upgraded embedded kernel passed 35 tests (19 kernel/lifecycle,
  13 collector fixtures, one typed-exception unit test and two transport tests),
  all-target Clippy and rustdoc with warnings denied, and the unchanged dependency
  policy. Wire fixtures cover the SDK's changed first-frame boundary; final
  receipts accompany the paired patches.
- The `claria-bedrock` dependency tree contains only `dekopon-embedded` from Dekopon;
  it adds no gateway, broker, Wasmtime, shell, OpenTelemetry, reqwest or ureq.
- Independent non-UI review verified completion-phase usage, typed exceptions,
  cache replay, reused tool IDs, receipt/effect ordering, fatal sibling accounting
  drain and per-invocation SDK attempt counters. No blocking source finding remains.
- The upgraded workspace passed 737 tests across all 13 non-desktop packages,
  with seven existing tests ignored: 326 service tests in
  `/tmp/writer-upgraded-services-tests.log` and 411 remaining-package tests in
  `/tmp/writer-final-v3-verified-tests.log`. The latter includes the first-frame fix,
  seven cleanup/claim fault cases, six process-death/completion-proof cases and
  a raw first-event transient-exception recovery case.
  Native transcription dependencies used local CMake 3.31.10. The failure receipt
  before the first-frame correction is retained separately.
- Actual-source credential tests passed 13 test entries, including 12 isolated
  subprocess scenarios, and all-target Clippy. Independent source review verified
  explicit-profile signing, no HTTP for a missing profile, and preserved default
  and inline behavior. See [dependency notes](writer-dependency-options.md).
- The React Writing page passed 11 browser flows, 56 component tests, build and
  lint. A new 600-pixel-window regression exercises review findings, real
  Keep/Rewrite clicks and resume. An independent reviewer found a second layout
  obstruction when findings and history coexist; the correction and revised
  screenshots await owner approval before re-review. No stylesheet, framework,
  generated binding, desktop command or `lib/tauri.ts` change was made.

All 13 non-desktop packages also passed all-target Clippy with warnings denied
(`/tmp/writer-final-v3-verified-clippy.log`). Nightly formatting passes all 27
changed Rust files across the pair. The full Claria formatting check reports six
files unchanged since the recorded baseline: `claria/src/lib.rs`,
`claria/tests/planning.rs`, `claria-desktop/src/config.rs`, and the eval package's `src/grade.rs`, `tests/config.rs` and `tests/grade.rs`. They were left outside
this isolated work; `/tmp/writer-final-v3-fmt.log` records that existing gate failure.

These checks do not cover the whole native desktop workspace, remote CI or a
released artifact. The inherited native Dekopon workspace was not rebuilt: its
source, manifest, lockfile and contracts are unchanged.

The new Claria fixtures are in `tests/stop.rs`, `tests/parallel_draft.rs`,
`tests/writer_backend_failure.rs`, `tests/writer_durability.rs`,
`tests/writer_crash.rs` and `tests/crash_boundary.rs` under
`crates/claria`. They check known/unknown accounting, exact Unicode content and
proposal effects, byte-identical retry inputs, durable receipts before repair,
accepted batch completion, no next call after Stop, and resume from stored sections.
The fatal-drain fixture directly covers an in-flight sibling repair and a receipt
failure; genuinely queued branches and a ready successful sibling after a fatal
error are source-reviewed, not separately fixture-tested. The historical dependency
probe used the older 23-test checkpoint; current upgrade receipts are separate.
Chat and structured-analysis startup cancellation and their broad classification
of first-frame exceptions retain their inherited behavior. Writer calls use the
embedded cancellation boundary and typed stream errors.

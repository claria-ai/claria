# Writer harness checkpoint

This is a local, synthetic-validation checkpoint, not a completed Writer replacement.
The starting Claria `main` was `bdd3731491b71f3ef87a530c19d220a324567230`.
The inspected Dekopon `main` was `0d4f4276071cf41fa36808eea0cd9b8d2d13bf5f`.
The local embedded-package checkpoint is `94acc810f6e96cb6f48ec10065fdab5f0c8a788d`.
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

The existing sequential executor and planned parallel executor are distinct.
Their retry budgets, finalization cuts and receipt ordering must survive later
loop migration. They remain in place in this first transport slice. The new
embedded loop is exercised independently with synthetic tool hosts; that alone
does not demonstrate better real-model writing.

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
The owner will provide test access later. Broader Writer loop migration waits for
parent review of this checkpoint.

The separate package's dependency audit currently fails: Claria's existing exact
Tokio 1.52.3 pin is affected by RUSTSEC-2026-0386, and the native Dekopon duplicate
crate policy rejects the AWS SDK's `http`/`http-body` generations. No exception or
uncoordinated dependency bump was added. This needs a parent-reviewed dependency
decision alongside the active release work.

The existing stopped-attempt path can retain `usage_complete=true` after an
in-flight call is discarded. This inherited accounting ambiguity is not fixed by
the faster Stop behavior. The new post-terminal failure path is separately tested
to retain unknown usage and avoid automatically repeating a completed call.

## Observed checkpoint evidence

On Linux with Rust 1.98.1:

- The unchanged baseline passed 264 tests; six existing live tests stayed ignored.
- The integrated `cargo test -p claria-bedrock -p claria --locked --no-fail-fast`
  passed 268 tests with the same six ignored tests. The four new cases cover exact
  streamed reasoning/tool replay, first-frame cancellation, a post-terminal broken
  trailer with durable unknown usage, and retry of a typed stream exception.
- `cargo clippy -p claria-bedrock -p claria --all-targets --locked -- -D warnings`
  passed. A pre-existing late-initialization warning under this compiler was fixed
  in the touched writer context initializer without changing its values.
- The separate embedded package passed 23 tests, all-target Clippy with warnings
  denied, formatting and rustdoc with warnings denied.
- The `claria-bedrock` dependency tree contains only `dekopon-embedded` from Dekopon;
  it adds no gateway, broker, Wasmtime, shell, OpenTelemetry, reqwest or ureq.
- Independent non-UI review found and verified fixes for completion-phase usage,
  typed stream-exception classification, cache replay and reused tool IDs. No
  introduced blocking source finding remains for this checkpoint.
- No frontend, layout, stylesheet, generated binding or desktop command changed.
  No browser/UI review was performed.

These checks do not cover the whole native desktop workspace, remote CI or a
released artifact. The inherited native Dekopon workspace was not rebuilt: its
source, manifest, lockfile and contracts are unchanged.

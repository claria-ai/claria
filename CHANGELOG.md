# Changelog

All notable changes to Claria are documented here.

## [Unreleased]

- Four high-severity advisories in the frontend dependency tree are patched, without moving any declared version range
- Tagged release builds pin the Tauri command-line tool and the release action to exact versions, so an upstream release can no longer change how a build is produced
- The two build dependencies that need install scripts are approved explicitly, and the approval is recorded in version control
- The bundle size warning threshold now reflects a bundle the app loads from local disk rather than over a network
- Builds no longer print funding notices on every dependency install
- The Tauri JavaScript API is pinned exactly, so the half of the pair that talks to the Rust runtime can no longer drift on an unrelated install
- Local transcription's settings file and model downloads are restricted to the account Claria runs as on Windows, where they were previously left at whatever permissions the folder handed out
- Restricting a file to the current user now fails loudly on every platform instead of reporting success it did not achieve

## [0.26.0] — 2026-08-15

- Chat and writer streams now tolerate five minutes of Bedrock silence before failing, up from two, so a cold prefill of a large record set no longer aborts the turn
- A writer Bedrock call whose connection breaks before a response completes is retried up to twice before the turn fails
- When every retry goes unanswered, the writer error now reports the attempt count and how long Bedrock was silent, and says to try again later
- Pull-request CI compiles, lints, and tests the Rust workspace on Windows alongside Ubuntu, so Windows-only code is validated before a release tag rather than after

## [0.25.0] — 2026-08-15

- Writer turns stream from Bedrock instead of waiting on one unary response, so a long generation no longer risks an HTTP timeout at the writer's output ceiling
- A chat reply cut off at the output limit keeps the text the reader watched arrive and says why it stops there, instead of discarding the whole answer
- Chat answers can run to a full clinical section before hitting the output limit, matching the writer's ceiling
- An exhausted writer guardrail now reports the tool-use rounds and Bedrock calls it reached, names the Preferences field that raises the one that bound, and warns that raising it costs more
- A writer request that is already at a guardrail's maximum says so and suggests narrowing the request instead
- Bedrock failures during a writer turn name the call that failed and its cause, so denied model access, throttling, and unreachable endpoints no longer read identically
- A saved configuration that cannot be loaded now reports why, instead of telling the clinician to complete setup and inviting them to overwrite it
- Running an older Claria against a newer configuration says the build is out of date and to update
- Each chat request and writer turn logs the context window resolved for the model alongside the input budget and output reserve derived from it
- AWS calls run on an explicit timeout policy: a short connect timeout, a bounded wait for response headers, and no cap on total call duration, so a long generation or a large upload is never cut off for taking time
- Stalled-stream protection is configured explicitly rather than inherited, with a grace period chosen for clinic networks
- A Bedrock response stream that goes silent mid-generation now fails with the reason instead of waiting forever
- Chat replays a bounded slice of a long conversation to Bedrock instead of re-uploading the whole thread on every turn; the conversation on screen and in the cloud stays complete
- Chat caches its prompt for an hour on models that support it, so a conversation resumed after seeing a patient reads from cache instead of paying full input rates again

## [0.24.0] — 2026-08-13

- Writer and chat context no longer rewrite angle brackets and ampersands inside clinical text; only sequences that could forge the untrusted-context delimiters are neutralized
- The writer's report context is structured, indented JSON again instead of one compacted line
- Writer proposals can carry 25 operations and 200 blocks per section again; the tool schema's ceilings now mirror the domain validators instead of maintaining smaller copies
- Writer responses get a four-times-larger output budget, and the proposal tool no longer instructs the model to keep proposals small
- Template exports keep blank spacer paragraphs aligned with the report instead of piling them near the top when the report outgrows the template
- Template exports no longer underline or bold generated paragraphs with formatting copied from an unrelated template line such as a signature blank or field label
- Template exports keep bullet lists numbered when the report also contains multi-line paragraphs
- Template import and export both recognize custom-named heading styles through the package's style definitions, so section headings keep the template's heading formatting and body text keeps the body font
- Filling an empty template table cell regenerates the table instead of silently dropping the generated value
- Multi-line paragraphs in template exports render their newlines as Word line breaks instead of disappearing
- Bullet lists exported through a template without its own list formatting carry their numbering definition into the package instead of referencing one that does not exist
- Exports that could not apply the imported Word template's formatting now say so in the export status instead of silently producing a default-formatted document
- Audit events record the app version and, for AI turns, the stop reason
- Writer per-call usage records add stop reason, latency, the output ceiling in effect, a system-prompt digest, and the app version; chat history messages add stop reason and latency
- The turn-complete console line reports stop reason, latency, and the output ceiling for every AI call
- The document writer's system prompts are editable in Preferences with versioned history, while the untrusted-data and template-carryover trust rules stay fixed and are shown read-only
- Preferences gains opt-in model tuning — adaptive reasoning, effort, and temperature — with each knob sent only to model generations the capability table says accept it
- A design/ folder documents the writer workflow and the template system

- The scoped Claria IAM policy no longer grants permanent S3 object-version deletion
- Full infrastructure teardown requires temporary elevated credentials from the configured AWS account
- Successful full infrastructure teardown removes the now-invalid local system configuration
- Tagged releases update the download site only after every desktop artifact is published
- Older release reruns cannot roll the download site back from a newer version
- Marketing screenshot dates are fixed so release-site reruns do not create time-only image changes

## [0.23.1] — 2026-08-12

- Writer responses cut off by the output-token limit mid tool call no longer fail the turn; the completed tool calls are executed and the model continues where it was cut
- The writer's corrective round for an inconsistent stop reason now re-arms after each well-formed response instead of being spent once per turn

## [0.23.0] — 2026-08-11

- Writing can fill an entire report in one action, save it directly as one versioned draft, and reserve reviewable proposals for later targeted edits
- Writing sessions now behave like chats: opening the Writing tab starts a fresh report, while Editor History resumes a specific prior session
- Whole-report generation loads every readable client record into one bounded source snapshot and drafts sections through internal cached tool rounds without asking the user to drive record retrieval; its prompt disappears after the first completed turn
- Writer context now keeps a text-free per-file provenance list for whole-report snapshots, adds files read by later tool turns, and opens those pills through the same preview component as Chat
- Whole-report replacement uses an in-app confirmation, reports failures beside the action, and logs request, failure, and completion diagnostics without record content
- Webview `console.error`/`console.warn`, uncaught errors, unhandled rejections, and React render crashes now flow into the Claria Console and rolling logs with useful stacks but without serializing arbitrary objects
- Reloaded chats and Writing sessions can reuse an exact five-minute prompt-cache prefix through small in-memory LRUs, and cache labels only claim expiry when elapsed time proves it
- New Writing sessions open on an optional setup tab for choosing a Word template or filling the whole report; a second tab starts tool-driven work, and a compact lightning/dollar tab contains session usage
- Chat and Writing keep cost and prompt-cache details in the same focused usage-tab design, including cache-write tokens and fees, rather than placing spend banners in the main flow
- The usage tab can opt into per-turn cost badges, while its full-width breakdown explains component spend, cache reuse, stale windows, and cold starts without a squeezed accordion
- Chat requests now end with a cache point on the conversation tail, so each turn re-reads the whole history from cache instead of paying full input rates
- Chat cache entries use the default five-minute tier and a hash-only in-memory prefix tracker rather than paying the doubled hour-long write rate
- Costs now price hour-long cache writes at their real 2× rate, and each turn records the cache TTL it used so historical totals stay honest
- Desktop commands are split into per-domain modules sharing one command context and one rich error type, with every error logged and stringified exactly once at the command boundary with its operation name
- The S3 client is cached alongside the SDK config and invalidated with it, instead of being rebuilt on every command
- S3 failures now keep their full connection-level error context instead of collapsing to "unhandled error", and storage no longer double-logs errors it already returns
- S3 timing spans log a scrubbed key class instead of client-chosen filenames, which are PHI
- Client names and record filenames no longer appear in application logs; the durable audit trail in S3 keeps the full identifiers
- The audit event's console mirror is now a one-line summary without the details payload, under a dedicated log target
- The exported console log is written with owner-only file permissions
- Logs also roll to bounded daily files on disk, and the console gains a button that opens the log folder
- Log filter directives for all workspace crates are built from one shared list
- Creating, renaming, deleting, and restoring clients, uploading, editing, deleting, and restoring record files, and saving prompts now all write durable audit events
- The app window enforces a strict content security policy allowing only bundled resources
- Assumed-role secrets and freshly minted access-key secrets no longer cross the IPC boundary; the frontend holds an opaque handle and the backend keeps the credentials
- Opening a URL on Windows goes through the platform API instead of a shell command, and URLs with whitespace or control characters are rejected
- Creating the IAM policy without a resolved account ID now fails instead of silently widening resource scopes to a wildcard
- The plaintext-with-file-permissions credential storage model is documented, with keychain migration tracked
- The provisioner receives its local state directory from the desktop instead of deriving app paths itself
- Both upload paths share one skeleton and one content-type map, differing only in whether a failed sidecar fails the command
- Restoring a deleted file uses the race-safe conditional restore, and record-file and prompt version listings, reads, and restores share one implementation
- Report writer turn audit events build their usage fields through the shared helper and no longer claim complete usage when no attempt ran
- Numbered default names share one generator, the prompt-cache settings shrink to the fields actually used, and the writer call ceiling is derived from the round ceiling
- Restore commands no longer take an ignored version parameter
- Report revision listings fetch uncached versions concurrently and remember per-version summaries, so revisits cost no reads
- Chat history listings revalidate against the record cache and fan out concurrently instead of one serial read per chat
- Record uploads stream file bodies from disk instead of buffering whole files in memory, reading bytes only when extraction or text validation needs them
- Conditional S3 writes and prefix listings share one implementation each, and loading a stored JSON object with validation is a single storage helper used by every reader
- Unused presign, first-version, and legacy token-cost helpers are removed along with error variants nothing raised
- Restoring a deleted client's files now runs restores concurrently, attempts every file even when one fails, and reports exactly which restores failed
- Client CRUD, record inventory, content search, the record cache, and the delete/restore lifecycle move into one records crate with structured errors, and the sidecar-visibility rules are implemented exactly once
- The audit-trail events and S3 sink fold into the storage crate, and reading a day of audit events fetches concurrently instead of one object at a time
- Model capabilities (tool support, context window, prompt caching, token-counting model) resolve through one central table, so new Claude generations get modern behavior — including prompt caching — by default
- All AI calls share one plumbing layer with structured error reporting, and turns whose token usage the service omitted are recorded as unmetered instead of zero-cost
- Every AI call now enforces an output-token ceiling: a cut-off chat, writer, translation, or document-extraction response fails with a clear error instead of silently saving truncated text
- Chat checks the conversation against the model's context window before sending and reports overflow with the same guidance the writer gives, instead of a raw AWS error
- Writer proposals are capped to sizes that fit one response, with large rewrites split across turns
- Writer tool errors now tell the model exactly what was malformed, and tool descriptions spell out ID copying, 0-based positions, character units, per-turn limits, and pagination
- Transcript translation receives its results through a forced structured tool call, verifies every segment came back, and no longer pins sampling temperature
- A writer turn whose final reply is cut short no longer discards an already-staged proposal, and a glitched stop reason gets one corrective retry before the turn fails
- Writer turns cache their prompt prefix on caching-capable models and count tokens once per turn instead of before every call, cutting cost and latency of multi-step turns
- The writer's system prompt is organized into headed sections and the untrusted report context travels as compact JSON inside named delimiter tags
- Chat record context is escaped so document text cannot forge its delimiters, placed after the instructions, and always followed by a fixed do-not-follow-instructions rule
- Document extraction sends a neutral user turn so the customizable extraction prompt is the single source of instructions
- Project instructions gain review-derived coding rules covering reuse, frontend patterns, Rust conventions, LLM calls, logging, security, and performance
- Chat and writer composers both send on Enter and insert a newline with Shift+Enter, with a native resize grip replacing the chat drag handle
- Writer activity cycles through clearer thinking, working, and inferring labels across internal model rounds
- Report bullet lists render with the intended compact spacing
- An unexpected interface crash shows the error with a reload button instead of a blank window
- Chat, writer, preferences, cost, and provisioning screens share one set of interface primitives, async-load handling, and state hooks without behavior changes
- The app ships a single modern TLS stack, verifies update checks and model downloads against the operating system's certificate store, and builds a smaller release binary
- Background interface failures are forwarded to the backend log stack instead of being silently swallowed, so they appear in the console window, saved log exports, and the on-disk log files
- The console window renders only the newest lines by default and polls with a sequence cursor that receives only new lines, instead of re-shipping the whole buffer twice a second
- Preference saves send only the changed section's fields and merge into the cloud copy under an ETag precondition, so sections and machines can no longer clobber each other's settings
- The report domain types cross the IPC boundary directly instead of through a field-for-field mirror layer, and one chat role enum replaces the three that existed
- Chat responses stream into the conversation as the model writes them, in both record chat and infrastructure chat, with truncation still surfacing as an error instead of silently saved partial text

## [0.22.0] — 2026-08-10

- Chat and writer sessions receive numbered names that can be edited directly in their panes and are shown in history
- Writing shows live and persisted context in one collapsible pill list, opens record contents from those pills, and uses a reusable activity throbber while the report agent plans, reads records, and drafts proposals
- Writing previews complete earlier report revisions in a scrollable Word-style view and restores any prior version as a new revision without deleting history
- Printable UTF-8 record files such as Markdown, JSON, CSV, and extensionless text remain directly previewable and readable by Chat and Writing while PDF and DOCX extraction produces structured Markdown
- Preferences includes a managed, renameable shelf of redacted Word writer templates with size, upload date, and best-effort usage counts
- Writing applies managed templates directly, locks the chosen template to its session, shows the applied template name, and removes responsibility popups, review gates, and dismissible warning banners
- Template-backed Word exports retain the source package's fonts, run styles, paragraph spacing, blank paragraphs, page setup, headers, footers, media, and table formatting
- A release helper regenerates marketing screenshots, updates website artifact metadata, and rebuilds the generated release site

## [0.21.0] — 2026-08-09

- Document writer loop and history limits are configurable in Preferences with ten-times-higher defaults and explicit cost and runaway-loop warnings

## [0.20.0] — 2026-08-09

- Record Memo transcription now runs locally through transcribe.cpp with verified, downloadable GGUF models
- Preferences expose machine-local model management, compute backends and devices, CPU/K/V controls, and advanced Whisper decoding settings
- Imported audio recordings continue to use Amazon Transcribe with speaker, language, medical, and translation options
- Removed the legacy Candle-based Whisper crate and safetensors model runtime
- The Apple Silicon app targets macOS 11 or later
- Metal builds include the macOS platform-availability runtime
- Client record settings can rename a record and show file counts, current and historical storage, creation dates, and name history
- The client record page is split into components and hooks, with each feature owning its own state
- The voice memo capture engine is a reusable hook rather than a page component's internals
- Leaving the Record tab during a voice memo stops the microphone, transcription loop, and audio context
- Record files and custom prompts share one version history dialog instead of a copy each
- Duplicate date and file-size formatters in the preferences page are gone
- Audio buffer handling, transcription summaries, record filename rules and the webview drag-and-drop listener have tests

## [0.19.0] — 2026-08-02

- Client workspaces have a new opt-in Writing tab for interactive report authoring
- Writing responses and accepted report content render Markdown formatting
- Reports edit inline without textarea cards, and unsaved edits are saved into the assistant's next message automatically
- Unsent Writing instructions survive in-app navigation, and no longer trap the client back control
- Writing tool loops accept empty companion text and transient Bedrock reasoning blocks without persisting private reasoning
- Writing falls back to the active Haiku tokenizer when a newly launched selected model does not support CountTokens
- Report paragraphs and tables can be attached to a Writing message from their hover controls
- Writing proposals show only net title, section, and changed-block differences instead of repeating the complete report
- Writing imports bounded DOCX templates through an explicit structured-content preview without retaining source files, filenames, or local paths
- Template-derived reports require a revision-specific carryover review before Word export
- Reports support editable plain-text tables with header rows, optional column widths, model proposals, and Word export
- Table proposals highlight changed cells while retaining surrounding row and column context
- Tool activity rows expand into collapsed-by-default raw LLM invocation and correlated-result JSON
- Writing exposes the accepted report and record excerpts read during the session in a context control
- Writing and local-export notices can be dismissed
- The native application menu restores standard copy, cut, paste, and select-all behavior in the desktop webview
- Word export uses an asynchronous native save dialog, remains retryable after cancellation, and persists its latest status
- Persisted writing sessions appear under Editor History on the client record
- The report assistant uses Bedrock Converse tools to list records and read bounded text from the current client
- Report token budgeting uses foundation model IDs while Converse continues through cross-region inference profiles
- Record text stays in the active tool loop while persisted activity retains only ranges, hashes, and safe status metadata
- Model-authored report changes remain reviewable proposals until the user explicitly accepts and saves them
- Accepted reports are structured, revisioned, optimistic-locked, and synced through S3 separately from records and chat history
- Bedrock usage and completion status are retained per call even when a report turn aborts or conflicts
- Manual report editing supports ordered sections, paragraphs, bullet lists, and tables without giving the model direct write access
- Pending report proposals survive tab switches and app restarts, and follow deleted clients through restoration
- Restoring a deleted client leaves its record files and text-only chat history deleted while recovering the opt-in report workspace
- Client deletion compensates partial failures and retries without overwriting concurrently restored report work
- Accepted report revisions export as genuine Word documents with headings, paragraphs, real bullet numbering, and styled tables
- Local Word exports disclose their PHI risk and use atomic writes with private file permissions
- The existing text-only Chat workflow remains unchanged and does not opt into report tools
- End-to-end browser checks can use an alternate local Vite port when port 1420 is already occupied
- Tearing down AWS infrastructure now deletes the storage bucket, which previously always failed because version history and delete markers were left behind
- The IAM policy grants permission to delete object versions, without which a teardown stops partway through
- Bulk deletes go out in batches of a thousand objects per request instead of one request per object
- A bulk delete that S3 partially refuses now reports an error naming the objects left behind instead of reporting success
- A teardown that fails partway through is safe to re-run
- Starting the desktop app in development installs frontend dependencies first, so a fresh checkout no longer fails on a missing Vite binary
- The mock AWS server refuses to delete a bucket that still holds objects
- The mock AWS server supports batch deletes, versioned deletes, and paged version listings
- Audit events are written to durable storage in the Claria bucket instead of only reaching an in-memory log that dies with the process
- Audit events carry the token-usage and cost payload that was previously built and then discarded before it was logged
- Every audit event has its own identifier and UTC timestamp, and lands under a year/month/day path so an auditor can list a single date
- Each event is one immutable object, so nothing recorded is lost if the app is killed
- An audit write that fails is reported in the Claria Console instead of failing the chat turn or upload that produced it
- The audit crate has tests
- The frontend has a unit test suite
- The transcript body format is pinned by fixtures the Rust and TypeScript parsers both read, so a change to one that the other misses fails on both sides
- A recording longer than an hour and forty minutes opened in the transcript editor as one un-diarized block, and saving it overwrote every speaker header
- Text typed above a diarized transcript no longer hides the segments below it in the editor
- Escape, backdrop clicks and scroll locking on the modal shell are covered by tests, in a simulated DOM and in a real browser
- The first-run start screen explains that no local configuration was detected instead of showing a bare Create New System button
- Onboarding copy on the Create-an-AWS-Account step points to the Next button instead of a Skip control that does not exist
- The MFA-step skip control renders as a bordered secondary button matching the design system instead of bare amber text
- Back on the AWS Infrastructure screen steps to the credential form during first-run setup, preserving entered values instead of resetting to the start screen, and is disabled while a scan or apply is in flight
- Token counting runs against the newest available Haiku instead of the chat model, which may not support the CountTokens API
- Chat surfaces no longer name a model when counting context tokens, so the count cannot drift as models are added to the account
- The context token count starts as soon as the context loads instead of waiting for the model list
- Editing the context while a count is in flight no longer lets the superseded count land
- Model pricing matches by tier so new Claude releases (Fable, Opus 4.8, Sonnet 5) are priced without a code change
- When a model has no pricing entry the UI omits dollar figures instead of showing $0.00 or "cost unavailable"
- Client and record file search ignores accents, so "Luci" finds "Lucí"
- Removed the unused credential intake page
- Dependency refresh across the workspace and frontend clears 10 of 17 RustSec advisories and all npm audit findings; the rest are transitive with no patched upstream (wayland-scanner and AWS smithy legacy TLS)
- The onboarding e2e test covers the unified provisioning flow that replaced the old wizard pages
- Client list and record file list share one deleted-items section and history toggle; deleted clients show as rows instead of a second table
- AWS infrastructure scan, plan review, apply progress, and errors all render through one lifecycle widget
- The AWS plan review is one flat list of all resources with drift diffs inline — no more sections with in-sync resources collapsed under Ready; scanning and applying render the same list with per-resource progress
- The IAM policy appears as a first-class resource in the plan list with its permission diff inline; the elevated-credentials notice is a slim banner linking down to the single apply button instead of duplicating the CTA and diff
- Removed two unreachable AWS management pages left over from the old onboarding wizard
- Release tag builds reuse the main-branch warm cache instead of recompiling ~60 dependency crates, cutting the macOS release build from ~11 to an expected ~7 minutes
- The release workflow caches npm downloads
- The mock AWS test server builds clean under the latest Clippy
- CI pins the Rust toolchain so a new stable release can't fail the build unannounced
- Removed the abandoned template-driven report-generation feature set: three crates, their domain models, and their S3 key layout
- Builds no longer compile tantivy, tera, or ts-rs
- The update check compares releases as semantic versions, so users on 0.9.x are no longer told they are current
- A transcription job that never finishes surfaces a timeout error after thirty minutes instead of freezing the app indefinitely
- Transcription status polling and IAM credential propagation both back off exponentially with jitter instead of polling on a fixed interval
- The deleted-clients list fetches names concurrently instead of two serial round-trips per entry
- The S3 bucket name, provisioner state key, and transcription output key all come from the shared key module instead of being rebuilt at each call site
- Object listings use the AWS SDK's own pagination instead of hand-rolled continuation-token loops
- AWS error messages come from the SDK's error-chain formatter rather than a local reimplementation of it
- An ETag optimistic-lock miss is detected from the HTTP status and error code instead of matching on error text
- Setting up a second computer against an already-provisioned AWS account offers a Set Up This Computer button instead of stranding the user on a plan with nothing to apply
- A conformant plan on a configured machine offers Continue to Claria
- Setting up a computer when AWS refuses a new access key reports the actual AWS error code and message instead of the words "service error"
- Setting up a third computer against an account whose IAM user already holds two access keys lists those keys with their creation and last-used dates and offers to delete one, instead of dead-ending
- Deleting an access key warns that the computer using it loses access to Claria and requires a per-key confirmation; nothing is preselected
- IAM and STS errors from the mock AWS server carry their error code, so tests exercise the same parsing path as the real SDK
- The icons pasted into pages by hand — close, back, trash, search, play, folder — come from one shared set
- Page back buttons and icon-only close buttons carry an accessible name for screen readers
- Every loading spinner in the app is now the same component instead of four copies of it
- The chat context token badge is defined once instead of separately in the client and infrastructure chats
- Every dialog in the app is a real modal dialog: Escape closes it, keyboard focus stays inside it, and the page behind it no longer scrolls
- Screen readers announce dialogs as modal and read their title
- Escape does not discard a just-recorded memo transcript, and clicking the dimmed background still does not dismiss a delete confirmation
- The thirteen hand-rolled modal overlays are one shared component
- Comparing two versions of a long transcript no longer stalls the window while a quadratic table is built; a 5,000-line pair diffs in milliseconds instead of half a second
- Every backend call the frontend makes goes through the generated bindings, so renaming a Rust command fails the build rather than the running app
- Transcription preferences save a moment after you change them rather than when you leave the screen, so quitting the app no longer discards the edit
- A transcription preference that fails to save says so and offers to retry, instead of failing silently into the log
- Typing a custom date range in Cost Explorer no longer bills a $0.01 lookup per keystroke; the range loads when you press Apply
- Cost Explorer permission and data-availability errors are recognised in one place instead of three
- A failed text extraction in the chat context bar shows a dismissible banner instead of a blocking system alert
- The Claria Console keeps updating once its log buffer is full, where it previously froze because rotation leaves the entry count unchanged
- Choosing a preferred chat model sticks, where it previously reverted to the old model as soon as the app re-read its settings
- Enabling Cost Explorer and turning on hourly cost data now follow you to your other computers instead of staying on the machine that set them
- Editing transcription preferences no longer reverts a preferred model or Cost Explorer setting changed earlier in the same visit
- Project instructions document the machine-local build environment and worktree cleanup
- Project instructions warn that a fresh worktree needs its frontend dependencies installed first, and name the misleading Ruby error that appears when they aren't
- The frontend unit tests run in CI
- CI installs frontend dependencies from the lockfile instead of re-resolving them

## [0.18.0] — 2026-07-07

- Record file search also matches the extracted text of each file, with a badge on files that only match by content
- The record file search opens from a magnifying-glass button, swapping the header action buttons for a wide search field while active
- Record file name matching is substring anywhere in the name, no longer prefix-only
- The client list can be filtered by name
- The AWS SDK config is cached across commands and all AWS clients share one HTTP connection pool, so navigation no longer pays DNS/TCP/TLS setup on every S3 call
- Missing synced preferences are seeded to S3 on first boot instead of being logged as a complaint on every view
- Transcription preferences sync automatically when leaving the Preferences screen, replacing the Save button

## [0.17.0] — 2026-07-06

- Record files can be searched by filename prefix; the search narrows the S3 listing itself rather than filtering client-side
- Client list and chat context load fetch their S3 objects concurrently instead of one at a time, so a ~100-record practice loads in well under a second instead of several seconds
- Bounded-concurrent S3 fetches and provisioner scans run through an ordered concurrent stream rather than a manual semaphore, preserving input order without a post-hoc sort
- Cache client and record reads in memory, refetching an object only when its S3 ETag changes
- S3 object operations, Bedrock chat turns, and the record/chat commands emit trace-level timing spans with elapsed milliseconds, keys, and byte counts
- Exported console logs carry those span durations regardless of log level, while the terminal and default operational log stay free of them

## [0.16.3] — 2026-05-20

- AWS SDK and IO errors are emitted as tracing events at their origin so the in-app Console surfaces them, not just the red banner
- Bedrock model-access provisioner verifies invocation entitlement instead of inferring it from the absence of marketplace offers — newly released Claude models no longer false-positive as "in sync"
- Provisioner tracks one Anthropic Claude family entry instead of per-version specs, so future Claude releases are picked up automatically

## [0.16.2] — 2026-05-19

### Added
- Transcribe Wizard now warns that AWS needs ≥30 s of speech for reliable language ID
- Bilingual-transcription acceptance test driving the SDK against an in-process mock with a real captured Mixed-mode cassette
- Debug example that runs a real Mixed-mode Transcribe job end-to-end and prints language-ID diagnostics
- Mock-AWS request capture and cassette playback for `StartTranscriptionJob`
- Test helper that binds the mock-AWS router to a loopback port for SDK-level tests

### Fixed
- Transcript segments now render in chronological order; AWS returns them grouped by speaker
- Mock-AWS S3 `Last-Modified` header now emits RFC 7231 HTTP-date so the AWS SDK can parse it
- Mock-AWS request-body limit raised to 64 MB so real audio fixtures can be uploaded through it

## [0.16.1] — 2026-05-18

### Fixed
- Release workflow couldn't find the frontend — Tauri 2 resolves `beforeBuildCommand` from the parent of the tauri crate, so `../../claria-desktop-frontend` was one level too deep. Reduced to `../claria-desktop-frontend`

## [0.16.0] — 2026-05-18

### Added
- Multi-lingual and Medical transcription — `claria-transcribe` accepts language, speaker count, and engine; returns structured segments with speaker, timestamp, and translation
- Spanish, English/Spanish code-switching, speaker diarization (1–4 speakers), and stereo channel identification
- Transcribe Medical engine for English with PHI tagging
- Optional Bedrock translation of non-English transcript segments to English (pinned to Sonnet 4.6)
- Cross-machine preferences sync via `_state/preferences.json` in S3 — preferred model, cost explorer, transcription defaults, prompt caching toggle
- Transcribe Wizard modal with per-file language / speakers / Medical / translation overrides
- Transcription Preferences UI section for the default values used by drag-and-drop uploads
- Per-segment transcript editor with speaker rename and editable translations; rollback via version history
- IAM coverage for Transcribe Medical actions
- `get_first_version` S3 helper returning the oldest non-delete-marker version of an object
- Per-turn token usage and computed cost recorded on every assistant chat message
- Bedrock prompt caching for chat (5-min TTL on system + record context), enabled by default — expected ~75% input-cost reduction on a typical session
- In-chat cost UI — per-turn badge, session total banner with cache-savings chip, last-turn footer, chat-history header
- Bedrock pricing entries for Claude Opus 4.5/4.6/4.7, Sonnet 4.5/4.6, and Haiku 4.5
- Automated demo video recording system (`demos/`) — three Playwright-scripted scenarios with WebM→MP4 conversion
- "See it in action" page template for claria-ai.github.io

### Changed
- Unified terraform-style reconciliation flow — bootstrap and resource provisioning now run in a single pass with lazy privilege escalation
- IAM User and IAM Policy resources changed from read-only to fully managed (create / update / destroy)
- New `CredentialScope` enum enables two-provider execution: admin credentials for IAM, scoped credentials for everything else
- Single unified Provision page replaces CredentialIntake, ScanProvision, and AwsManage
- Manifest version bumped to 7; config schema bumped to v7

### Fixed
- Translation now works when no preferred chat model is set — falls back to a pinned Sonnet 4.6 instead of silently no-opping
- Drift card for array-valued fields renders a single unified list with added/removed rows instead of striking out the entire expected array
- Eye-icon preview on audio files opens the structured transcript editor instead of the read-only `<pre>` view
- Version-history modal on audio files lists transcript revisions, not the immutable audio object
- Drag-drop onto the records page no longer fires twice under React 18 StrictMode
- Drag-drop onto the open Transcribe Wizard routes into the wizard's file slot instead of bypassing it
- Delete-file confirmation no longer says "This cannot be undone" — deleted files are restorable from the More-mode UI
- Replaced custom `build.rs` JS build with Tauri's built-in `beforeBuildCommand`, enabling Vite hot-reload during development
- `claria-desktop` build on `main` restored — missing `aws-sdk-iam` dependency added, removed `manifest_version` reference
- `cargo run -p claria-desktop` rebuilds the frontend when `package-lock.json` is newer than `dist/index.html`; opt out with `CLARIA_SKIP_FRONTEND_BUILD=1` or `CI=true`
- CI smoke-tests the built `claria-desktop` binary under `xvfb` and fails the build if the WebView terminates within 15 s of launch
- CI Tauri-build job: ~5–6 m → ~3 m via parallelization, mold linker, and apt-deps caching

### Dependencies
- Routine patch bumps: `tracing-subscriber` 0.3.22→0.3.23, `color-eyre` 0.6.3→0.6.5, `tempfile` 3.26.0→3.27.0, `tar` 0.4.44→0.4.45, `docx-rs` 0.4.19→0.4.20, `rfd` 0.17.1→0.17.2, `futures` 0.3.31→0.3.32
- `tokio` 1.49.0 → 1.52.3 (mpsc and RwLock soundness fixes)
- `jiff` 0.2.21 → 0.2.24 (IANA tzdb update to `2026a`)
- AWS SDK family lockstep bump: `aws-config`, S3, Bedrock, BedrockRuntime, CloudTrail, IAM, STS, Transcribe, CostExplorer, Artifact
- Tauri 2.10.2 → 2.11.2 (`tauri` crate, `tauri-build`, `@tauri-apps/api`)
- Frontend `npm update` within existing semver ranges; `eslint-plugin-react-hooks` held at `~7.0.1`
- `uuid` 1.21.0 → 1.23.1
- `ureq` 3.0.11 → 3.3.0 (MSRV raised to 1.85)
- `candle-core`/`candle-nn`/`candle-transformers` 0.9.2 → 0.10.2 (Metal backend improvements, NaN fixes for GGML quantized models)
- `tokenizers` 0.22.2 → 0.23.1
- `tantivy` 0.25.0 → 0.26.1 — `TopDocs::with_limit` is now a builder; local Tantivy index may need a rebuild

## [0.15.0] — 2026-03-04

### Added
- Chat context now shows ALL record files as context pills, not just those with extracted text — files without sidecars appear dimmed with a refresh button
- New `extract_record_file` command re-runs Bedrock document extraction or audio transcription on demand from the chat context bar
- Provisioner streams scan/apply progress to the frontend via `Channel<T>` with concurrent resource scanning (up to 5 at a time)
- Provisioner plan() test suite with MockSyncer
- File version history screenshot with diff view (dev tooling)

### Fixed
- Chat context loading errors are now surfaced in the UI instead of silently swallowed
- Chat context pills for record files without `.text` sidecars were invisible — now always shown
- Version history modal and diff panel enlarged for readability

## [0.14.0] — 2026-03-04

### Added
- Claria Console — in-memory ring buffer (10 MB) captures tracing logs; open via Help > Claria Console menu in a separate window with live streaming (500 ms polling), search with Cmd+F, level filters (ERROR/WARN/INFO/DEBUG/TRACE), Copy to clipboard, and native Save As dialog via `rfd`

### Changed
- Licensed under GPL-3.0-only (previously proprietary)
- Added Contributor License Agreement (CLA) for external contributions

## [0.13.0] — 2026-03-03

### Added
- Infrastructure chat — ask questions about your AWS resources, security configuration, and drift status using Bedrock with full infrastructure context
- Cost Explorer — view AWS spending by service with daily/monthly granularity, date presets, and on-demand data refresh ($0.01 per refresh)
- Context token counting — free Bedrock CountTokens API shows context size next to "Context:" label in both client and infra chat, with spinner while loading and error indicator on failure
- Removable context pills — click [X] on any context file pill to exclude it from the conversation; token count updates automatically
- About page links open in system browser (macOS `open`, Windows `cmd /c start`, Linux `xdg-open`)
- About page resource links: Claria-AI website, open source code, Anthropic system prompts, Claude prompting best practices
- `bedrock:CountTokens` added to IAM policy and manifest for drift detection

### Changed
- Context pills wrap instead of scrolling horizontally
- Chat commands accept `context_filenames` parameter so removed pills are excluded from both token counting and inference
- Extracted `build_infra_system_prompt()` helper for reuse between infra chat and token counting

### Fixed
- IAM policy syncer now detects extra actions as drift (not just missing ones)
- Drift comparison lifted from individual syncers into the framework for consistency
- Cost Explorer preset stays selected when switching granularity

## [0.12.0] — 2026-03-02

### Added
- Check for updates on the About page — shows a banner when a newer release is available on GitHub
- Playwright screenshot capture suite for automated landing page screenshots (dev tooling)

### Fixed
- Turbo model crash: added 128-bin mel filters required by whisper-large-v3-turbo (was using 80-bin filters, causing index-out-of-bounds panic)
- Language detection for turbo model: include added tokens when scanning tokenizer vocabulary
- Recover from poisoned whisper mutex after a panic instead of permanently failing
- Model info tooltip now shows the actual model name (e.g. `whisper-large-v3-turbo`)

## [0.11.0] — 2026-03-01

### Added
- Metal GPU acceleration for Whisper inference on macOS (Apple Silicon). CPU fallback when Metal is unavailable. Windows remains CPU-only — candle has no DirectX/Vulkan backend; cross-vendor GPU would require replacing the inference engine (e.g. ONNX Runtime with DirectML).
- GPU/CPU indicator pill and model info tooltip in the recording UI
- Auto-discover all supported languages from the Whisper tokenizer (~99 languages) instead of hardcoding English and Spanish
- Orphan model directory detection — Preferences shows unknown model folders on disk with size and a Remove button, so clinicians can clean up leftover downloads without migration logic
- GitHub release notes now auto-populated from CHANGELOG

### Changed
- Replaced Medium tier (~3 GB `whisper-medium`) with Turbo tier (~1.5 GB `whisper-large-v3-turbo`) — better accuracy, smaller download, faster inference
- Existing "medium" config values automatically map to the new Turbo tier

## [0.10.0] — 2026-03-01

### Added
- Configurable Whisper model tiers — choose between Good English (~293 MB), Good English + Spanish (~967 MB), or Very Good Spanish (~3 GB)
- Multilingual language detection for Spanish and English (auto-detected from audio)
- Language badge (EN/ES) shown in the recording UI when using a multilingual model
- Multiple models can be downloaded and cached on disk, with one active at a time

## [0.9.0] — 2026-03-01

### Added
- Record Memo — opt-in local audio transcription using Whisper (candle, pure Rust). Record from the microphone, see words appear live, pause/resume/edit, and save as a `.txt` note. Audio never leaves the device.
- New `claria-whisper` crate wrapping candle for on-device Whisper inference (CPU, English-only base model)
- Whisper model management in Preferences — download (~293 MB), view status, or remove the model
- macOS microphone usage description (`Info.plist`) for app bundle signing

### Fixed
- Deleted `.txt` memos no longer show phantom sidecar duplicates in the deleted files list

## [0.8.0] — 2026-03-01

### Changed
- Migrated prompts to `claria-prompts/` S3 prefix — system prompt moved from `system-prompt.md` to `claria-prompts/system-prompt.md` with auto-migration of legacy key on first access
- PDF/DOCX extraction prompt is now customizable via Preferences (stored at `claria-prompts/pdf-extraction.md`)
- Generalized prompt commands: replaced system-prompt-specific Tauri commands with generic `get_prompt`/`save_prompt`/`delete_prompt` that accept a prompt name
- Preferences page now shows editable sections for both system prompt and extraction prompt with version history
- Updated default extraction prompt to preserve table structure as markdown

## [0.7.0] — 2026-03-01

### Added
- Preferences page with system prompt editor and chat model selection
- Chat context loading indicator — spinner and "Building context..." shown while record context is assembled; input disabled until ready

### Fixed
- HIPAA-compliant restore: restoring deleted files and clients now creates a new S3 version instead of removing the delete marker, preserving the full audit trail
- Retry loading chat models after onboarding completes
- Render markdown tables in chat with remark-gfm
- Clarify chat empty-state text and decouple from file lifecycle

### Changed
- Renamed dashboard view to "AWS" with back-arrow navigation consistent with other pages
- Renamed dashboard resource sections and added expandable resource details
- Removed `s3:DeleteObjectVersion` from IAM policy (no longer needed)
- Manifest version bumped to v4
- Rewrote README for clinician audience

## [0.6.0] — 2026-03-01

### Added
- Version history for record files — browse, view, and compare any two versions with character-level inline diff
- Deleted record recovery — restore deleted files and clients from S3 versioning
- "More" toggle on Clients list and Client Record pages to reveal version history and deleted items

### Changed
- IAM policy updated with `s3:GetObjectVersion` and `s3:ListBucketVersions`
- Manifest version bumped to v3

## [0.5.0] — 2026-03-01

### Added
- Audio transcription via Amazon Transcribe — drag-and-drop MP3, WAV, and other audio files to auto-generate text sidecars
- Client deletion with recursive S3 cleanup of all associated records, files, and chat history
- About page now reads version from Tauri metadata and links to website and GitHub

### Fixed
- IAM policy drift falsely reported after every apply — manifest iam_actions now match actual IAM action names

### Changed
- Simplified start screen layout: centered title, subtle top-right navigation
- Added CHANGELOG backfilled from v0.1.1

## [0.4.1] — 2026-02-28

### Fixed
- Load chat models once at app startup instead of per-request; sort history newest-first

## [0.4.0] — 2026-02-28

### Added
- Group chat history into collapsible folders with resume support

### Changed
- Publish GitHub releases automatically instead of as drafts
- Cancel in-progress main-branch builds when a new push arrives

## [0.3.0] — 2026-02-28

### Added
- IAM policy escalation flow accessible from the dashboard
- Resizable chat textarea with drag handle

### Fixed
- Bedrock model-agreement syncer falsely reporting pending status
- Warm Rust cache on main branch for faster release builds

## [0.2.1] — 2026-02-28

### Fixed
- Execute creates and modifies in manifest order; improve provisioner error messages

## [0.2.0] — 2026-02-28

### Added
- Improved onboarding flow with MFA guide and clearer provisioner labels

## [0.1.1] — 2026-02-28

Initial tagged release.

### Added
- Cargo workspace with core library crates (`claria-core`, `claria-storage`, `claria-search`, `claria-bedrock`, `claria-audit`, `claria-provisioner`)
- Tauri 2.x desktop app with React frontend
- Config persistence with versioned migration pipeline
- AWS credential assessment and IAM user bootstrap during onboarding
- Manifest-driven provisioner with scan → plan → apply lifecycle (S3, CloudTrail, Bedrock access, BAA check)
- Client record management with drag-and-drop file upload
- PDF/DOCX text extraction via Bedrock document processing
- Inline text file creation and editing
- Bedrock chat with dynamic model selection, hybrid model discovery, and automatic agreement acceptance
- Client record context injection into chat conversations
- Persistent chat history
- Customizable system prompt editor
- Markdown rendering for assistant messages
- GitHub Actions release workflow with `cargo-release` integration

### Fixed
- Eliminate UI flash on start screen by lifting config state
- Use `i32` for `RecordFile.size` to satisfy specta
- Enable drag-drop events and update extraction model
- Build frontend before Tauri build in release workflow
- Use `cmd /C npm` on Windows in `build.rs`

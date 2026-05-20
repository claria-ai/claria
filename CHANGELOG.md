# Changelog

All notable changes to Claria are documented here.

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

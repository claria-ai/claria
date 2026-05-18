# Changelog

All notable changes to Claria are documented here.

## [Unreleased]

### Added
- Automated demo video recording system (`demos/`) — three Playwright-scripted scenarios (bootstrap, cloud sync, record + chat) with Tauri IPC mocking and WebM→MP4 conversion script
- "See it in action" page template (`demos/site/demos.html`) for claria-ai.github.io

### Changed
- **Unified terraform-style reconciliation flow** — bootstrap and resource provisioning are now a single reconciliation loop instead of separate phases. IAM user/policy creation, credential handoff, and S3/CloudTrail/Bedrock provisioning happen in one pass with lazy privilege escalation
- IAM User and IAM Policy resources changed from read-only (Data) to managed (Managed+Elevated) — they can now be created, updated, and destroyed through the standard syncer interface
- New `CredentialScope` enum (Elevated/Regular) on `ResourceSpec` enables two-provider execution: admin credentials for IAM resources, scoped credentials for everything else
- Single unified Provision page replaces CredentialIntake, ScanProvision, and AwsManage pages
- Manifest version bumped to 7

### Fixed
- Replaced custom `build.rs` JS build with Tauri's built-in `beforeBuildCommand`/`beforeDevCommand`, enabling Vite dev server hot-reload during development
- `claria-desktop` build was broken on `main` — `commands.rs` referenced the removed `manifest_version` field on `ProvisionerState` and used `aws_sdk_iam` without declaring it as a dependency. `claria_provisioner::create_access_key` now takes `&SdkConfig` instead of `&aws_sdk_iam::Client`, matching the library-crate boundary rule
- `cargo run -p claria-desktop` now reinstalls and rebuilds the frontend when `claria-desktop-frontend/package-lock.json` is newer than `dist/index.html`. Plain `cargo run` previously skipped `tauri.conf.json`'s `beforeBuildCommand`, leaving the bundled `@tauri-apps/api` JS at a stale version after a Tauri bump and crashing the WebView on launch (`web content process terminated`). Set `CLARIA_SKIP_FRONTEND_BUILD=1` or `CI=true` to opt out
- CI now smoke-tests the built `claria-desktop` binary under `xvfb` and fails the build if the WebView terminates within 15 s of launch — catches Rust/JS Tauri API version drift before it lands on `main`

### Dependencies
- Routine patch bumps: `tracing-subscriber` 0.3.22→0.3.23, `color-eyre` 0.6.3→0.6.5, `tempfile` 3.26.0→3.27.0 (`TempPath::from_path` correctness fix), `tar` 0.4.44→0.4.45, `docx-rs` 0.4.19→0.4.20, `rfd` 0.17.1→0.17.2, `futures` 0.3.31→0.3.32 (drops `pin-utils`/`num_cpus` transitive deps)
- `tokio` 1.49.0 → 1.52.3 (mpsc and RwLock soundness fixes)
- `jiff` 0.2.21 → 0.2.24 (IANA timezone database update to `2026a`; signed-duration conversion panic fix)
- AWS SDK family lockstep bump: `aws-config` 1.8.14→1.8.16 (identity-cache memory leak fix when overriding credentials), `aws-sdk-s3` 1.124.0→1.132.0, `aws-sdk-bedrock` 1.133.0→1.141.0 (model lifecycle fields, Guardrails policy generation), `aws-sdk-bedrockruntime` 1.126.0→1.130.0, `aws-sdk-cloudtrail` 1.104.0→1.107.0, `aws-sdk-iam` 1.88.0→1.108.1, `aws-sdk-sts` 1.99.0→1.103.0, `aws-sdk-transcribe` 1.101.0→1.104.0, `aws-sdk-costexplorer` 1.111.0→1.114.0, `aws-sdk-artifact` 1.86.0→1.89.0, `aws-smithy-types` 1.4.5→1.4.7 (hardware-accelerated SHA-2), `aws-smithy-runtime-api` 1.11.5→1.12.0
- `bytes` 1.10.1→1.11.1 in `claria-mock-aws` (transitive requirement from the new `aws-config`)
- Tauri 2.10.2 → 2.11.2 (`tauri` crate, `tauri-build` 2.5.5→2.6.2, `@tauri-apps/api` ^2.10.1→^2.11.0). Regenerates `crates/claria-desktop/gen/schemas/*.json`
- Frontend `npm update` — refreshed within existing semver ranges: `react`/`react-dom` to 19.2.6, `tailwindcss` + `@tailwindcss/vite` to 4.3.0, `typescript-eslint` to 8.59.3, `vite` to 7.3.3, `eslint-plugin-react-refresh` to 0.4.26, `@types/node` to 24.12.4, plus assorted transitives. `eslint-plugin-react-hooks` pinned at `~7.0.1` (7.1.x introduces a strict `react-hooks/set-state-in-effect` rule that flags ~14 existing call sites — to be addressed in a follow-up)
- `uuid` 1.21.0 → 1.23.1 (default RNG switched to rand 0.10; `Version::Max` and `get_version` semantics tightened — audited, no call sites affected)
- `ureq` 3.0.11 → 3.3.0 (stable webpki-roots, `NO_PROXY` support, chunked-transfer/DNS-via-proxy fixes). MSRV raised to 1.85; current toolchain (1.94) is well above
- `candle-core`/`candle-nn`/`candle-transformers` 0.9.2 → 0.10.2 — Metal backend improvements (inter-encoder sync, concurrent dispatching, `StorageModePrivate` for intermediates, u64 seed-buffer size fix), NaN fixes for GGML quantized models, new `upsample_bilinear2d`. `DType` is now `#[non_exhaustive]` but Claria has no `match` arms on it
- `tokenizers` 0.22.2 → 0.23.1 — 96% faster added-vocabulary deserialization, 16% BPE batch-encoding improvement. `add_tokens` now normalizes content at insertion; Claria only reads `tokenizer.json` so the round-trip difference does not apply
- `tantivy` 0.25.0 → 0.26.1 — `TopDocs::with_limit(n)` is now a builder and no longer implements `Collector` directly; call sites in `claria-search/src/query.rs` updated to chain `.order_by_score()` to preserve the score-ordered behavior. Brings lazy scorers, faster intersections, HyperLogLog++, and a quadratic-time nested-aggregations fix. **Heads-up:** the on-disk index format compatibility between 0.25 and 0.26 is not explicitly guaranteed in the changelog. Users may need to rebuild their local Tantivy index; the cached `_index/tantivy.tar.zst` in S3 will be regenerated on next index refresh

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

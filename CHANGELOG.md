# Changelog

All notable changes to Claria are documented here.

## [Unreleased]

### Added
- **Bilingual transcription acceptance test** — `crates/claria-transcribe/tests/bilingual_acceptance.rs` drives `transcribe_audio_with_options` end-to-end against an in-process `claria-mock-aws` server, asserting on both the captured `StartTranscriptionJob` body the SDK sends (`IdentifyMultipleLanguages: true`, `LanguageOptions: [en-US, es-US]`, speaker diarization, `MediaFormat: mp4` — and the *absence* of a pinned `LanguageCode`) and the parsed `TranscriptResult` (segments carry both `en-US` and `es-US`, and each speaker stays mono-lingual). Audio fixture `tests/assets/133 Oak Hill Ave 3.m4a` is the opening of Don Quijote read once in Spanish and once in English by two different speakers; cassette `tests/assets/133-oak-hill-ave-3.transcribe.json` is the real AWS Transcribe Mixed-mode output for that audio (229.44 s en-US + 157.91 s es-US, 1107 individually-language-tagged items, 25 diarized speaker segments). Refresh by re-running `cargo run --example transcribe_m4a -- <bucket>` and overwriting the JSON.
- **`transcribe_m4a` debug example** in `claria-transcribe` — uploads a local audio file to S3, runs a real AWS Transcribe job in Mixed mode + 2-speaker diarization, downloads the raw transcript JSON, prints AWS's `language_codes`, a per-item `language_code` histogram, and the parsed segment list. Cleans up S3 artifacts by default; pass `--keep` to retain. Defaults the audio path to the bundled fixture so a quick smoke is `cargo run --example transcribe_m4a -- <bucket>`. Used to capture the real cassette and to diagnose AWS-side language-ID quirks on short recordings.
- **`claria_transcribe::parse_transcribe_json` is now public** so debug tooling and integration tests can share the production parser instead of reimplementing it.
- **`claria-mock-aws` request capture + cassette playback for Transcribe** — `MockState::transcribe_requests` records every `StartTranscriptionJob` body in arrival order; `MockState::transcribe_response_cassette` lets tests preload the transcript JSON the mock writes to S3 instead of the legacy hardcoded English stub. Existing transcribe tests are unchanged (the stub stays the default).
- **`claria_mock_aws::testing::MockServer`** — spawns the axum router on `127.0.0.1:0` and exposes the bound endpoint + shared state so SDK-level tests can build an `SdkConfig` with `endpoint_url` pointing at it. Aborts the serving task on drop.

### Fixed
- Transcribed segments are now sorted chronologically by `start_seconds`. AWS Transcribe returns `speaker_labels.segments` grouped by speaker (one speaker's turns listed contiguously, then the other's) rather than in time order, so a 2-speaker bilingual recording would render in the editor as "speaker A: 06:26, speaker A: 06:38, speaker B: 00:02, speaker B: 00:10, ..." — visibly jumbled. `parse_transcribe_json` now sorts and re-assigns sequential `seg_NNNN` IDs after building the segment list so `seg_0001` is always the first thing said. The acceptance test asserts on chronological order.
- `claria-mock-aws` S3 `Last-Modified` response header now emits RFC 7231 HTTP-date format (`Wed, 21 Oct 2015 07:28:00 GMT`) instead of jiff's ISO 8601 default. The AWS Rust SDK strictly parses this header and `unhandled error`'d on the old format the moment any test pointed a real SDK client at the mock. XML payload `LastModified` elements keep the ISO 8601 form they already had.
- `claria-mock-aws` router request-body limit raised from axum's 2 MB default to 64 MB so realistic audio fixtures can be PUT through the mock S3 path without `413 Payload Too Large` blowing up the test.

## [0.16.1] — 2026-05-18

### Fixed
- Release workflow couldn't find the frontend — `beforeBuildCommand` and `beforeDevCommand` were using `../../claria-desktop-frontend`, but Tauri 2 runs those commands from the parent of the tauri project directory (i.e. `crates/`, not `crates/claria-desktop/`), so the relative path was one level too deep. Reduced to `../claria-desktop-frontend`. `frontendDist` stays at `../../claria-desktop-frontend/dist` because that path is resolved relative to `tauri.conf.json` itself

## [0.16.0] — 2026-05-18

### Added
- **Multi-lingual + Medical transcription API** — `claria-transcribe` now accepts `TranscribeOptions { language, speakers, engine }` and returns a structured `TranscriptResult` (segments with speaker/timestamp/language/translation, speaker list). Supports Spanish (`es-US`), code-switching English/Spanish via `IdentifyMultipleLanguages`, Transcribe Medical (English-only, PHI tagging via `ContentIdentificationType=PHI`), speaker diarization (1/2/3-4 speakers), and channel identification for stereo audio. Engine is auto-routed: Medical falls back to Standard for non-English languages. `TranscriptSegment` carries whole-second timestamps (`start_seconds`/`end_seconds: u32`) — no false sub-second precision. New `format_transcript_body`/`parse_transcript_body` render and parse a `[Speaker mm:ss\u{2013}mm:ss lang]`-headered plain-text format so the existing `.text` sidecar carries structure through user edits; translations render as `> `-prefixed blockquote lines beneath the original; legacy header-less sidecars degrade to a single un-diarized segment. Legacy `transcribe_audio` retained as a thin wrapper during the transition window.
- **`SyncedPreferences` cross-machine sync (Rust scaffolding)** — split `ClariaConfig` into machine-local (region, credentials, account_id, system_name, created_at) and synced (preferred_model_id, cost_explorer_enabled, hourly_cost_data, prompt_caching_enabled, transcription) subsets. The synced subset is destined for `_state/preferences.json` in S3; Tauri command wiring follows in a subsequent commit. New `claria_core::s3_keys::PREFERENCES` constant. Config schema bumped to v7: v5→v6 migration injects `transcription` defaults (English, 2 speakers, Standard engine, no medical); v6→v7 adds `translate_to_english: false`.
- **Bedrock translation helper** (`claria_bedrock::translate::translate_segments`) — batched per-segment translation of non-English transcript text via a single Bedrock Converse call with a JSON-envelope response (`{translations: [{index, translation}]}`). Caller-agnostic shape: takes `(index, language_code, source_text)` tuples and returns translations by index so it stays independent of `claria-transcribe`. Returns a `TurnUsage` block for billing/audit.
- **Cross-machine preferences sync (Tauri wiring)** — `load_config` now overlays `_state/preferences.json` from S3 onto the in-memory config after the SDK is built; first-launch and read failures are non-fatal (local config wins, warning logged). New `save_preferences` command writes both the local config file and S3, bubbling S3-write failures with a "saved locally but cloud sync failed" message so the UI can surface the partial state. New `fetch_cloud_preferences` command re-fetches from S3 on Preferences-page entry so the editing machine sees its own latest values without an app restart. `ConfigInfo` now carries the `transcription` block so the frontend can read transcription defaults.
- **Transcription wizard Tauri commands** — `upload_record_file_with_options(client_id, file_path, overrides)` uploads + runs the new structured transcribe + optional Bedrock translation in one go. `save_transcript_edits(client_id, filename, body)` writes user edits to the `.text` sidecar (S3 versioning preserves v1). `restore_original_transcript(client_id, filename)` fetches v1 via the new `get_first_version` helper and PUTs it as the new latest, returning the restored body. New `TranscribeOptionsOverrides`/`SpeakerMode` typed payloads on the Tauri boundary. Audit events emitted for translate / save-edits / restore.
- **Drag-drop honours saved preferences** — `upload_record_file` and `extract_record_file` (re-extract path) now read `ClariaConfig::transcription` and call `transcribe_audio_with_options` with the user's defaults, plus optional Bedrock translation when `translate_to_english` is enabled. The hardcoded `en-US` Standard path is gone; the legacy `transcribe_audio` wrapper remains in the library crate during the transition window but no longer has callers in the desktop binary.
- **Transcription Preferences UI** — new section on the Preferences page covering default language (English / Spanish / Mixed), default speaker count, "Use Transcribe Medical for English" toggle with 3x-cost disclosure, and "Translate non-English to English" toggle. Persistent banner at the top of the page documents the cross-machine sync model and the restart-required behaviour for other open copies of Claria.
- **Transcription wizard (`TranscribeWizard.tsx`)** — separate "Upload Audio File…" entry point in the client records UI opens a modal with a native file picker (rfd-backed `pick_audio_file` command), per-file language radio, speaker stepper (1/2/3-4/Stereo channels), Medical override (English only), translation override, and a derived engine summary. Submit calls `upload_record_file_with_options`. Drag-and-drop stays untouched and uses preferences as-is, with a hover tooltip on the drag zone summarising current preferences and ETA.
- **Per-segment transcript editor (`TranscriptEditor.tsx`)** — replaces the read-only `<pre>` preview for audio sidecars. Parses the headered body into segments, exposes one editable row per segment, a Speakers rename pane (one row per `speaker_id`, propagates to every header on save), and an editable translation line beneath any translated segment. "Save" calls `save_transcript_edits` (S3 versioning records the new revision). Rollback flows through the standard Version History modal (relabelled "Transcript History" for audio files), which lists every transcript revision including v1 — no parallel one-click path. PDF/DOCX sidecars keep the old read-only preview.
- **IAM coverage for Transcribe Medical** — `account_setup.rs` and `manifest.rs` both grant `transcribe:{Start,Get,Delete}MedicalTranscriptionJob` alongside the existing Standard actions. Existing buckets need a reconciliation run to pick up the new actions; the manifest already drives that.
- **`get_first_version` S3 helper** — convenience in `claria-storage` that returns the oldest non-delete-marker version of an object. General-purpose utility for "give me the original version of this object" workflows.
- **Per-turn token-usage capture and persistence** — every assistant chat turn now records a `TurnUsage` block (model_id, input/output/cache tokens, computed `cost_usd`, `pricing_version`) onto the assistant `ChatHistoryMessage` in S3. Cost is reconstructable from chat-history JSON alone — no Bedrock or Cost Explorer round-trip required. New `claria-billing::pricing` table with dash-boundary family matching that kills the `id.contains("claude-opus-4")` substring trap (and consequently requires explicit entries for each minor generation, e.g. Opus 4.5). Audit events emitted per chat / infra-chat / extraction turn with UUIDs, `model_id`, integer token counts, and computed cost only — no message content (HIPAA-safe).
- **Bedrock prompt caching for chat (5-min TTL on system + record context)** — `chat_converse` places a single `cachePoint` block after the system prefix when caching is enabled and the prefix exceeds ~1,200 tokens. Tracing target `claria_bedrock::cache` emits per-turn `hit_rate`, `cache_read`, `cache_write`, and `cost_usd` for observability. New `prompt_caching_enabled` config flag (default `true`, config v5 migration); models that don't support caching silently fall back to the standard input price. Expected ~75% input-cost reduction on a typical 10-turn 5,000-token-context session.
- **In-chat cost visibility** — per-turn cost badge under each assistant bubble, with a hover tooltip showing token breakdown and cache hit rate; always-visible session total banner above the composer with token totals and a "saved $X via cache" chip; `LastTurnFooter` showing the most recent turn's spend above the composer; `ChatHistoryHeader` on resumed chats summarising lifetime cost, turn count, and last-activity time. Pre-flight estimate next to the spinner uses the new `lookup_model_pricing` command — no extra Bedrock round-trip. Graceful degradation throughout: legacy pre-tracking turns render "cost not recorded"; models without pricing render "cost unavailable"; never `$NaN`.
- **Bedrock pricing for Claude 4.5+ generation** — `claria-billing::pricing` now covers Claude Opus 4.5/4.6/4.7 ($5 input / $25 output, 3× cheaper than Opus 4), Sonnet 4.5/4.6 ($3 / $15), and Haiku 4.5 ($1 / $5). Cache prices follow the standard 0.10× read / 1.25× write multiplier. `PRICING_VERSION` bumped to 3.
- Automated demo video recording system (`demos/`) — three Playwright-scripted scenarios (bootstrap, cloud sync, record + chat) with Tauri IPC mocking and WebM→MP4 conversion script
- "See it in action" page template (`demos/site/demos.html`) for claria-ai.github.io

### Changed
- **Unified terraform-style reconciliation flow** — bootstrap and resource provisioning are now a single reconciliation loop instead of separate phases. IAM user/policy creation, credential handoff, and S3/CloudTrail/Bedrock provisioning happen in one pass with lazy privilege escalation
- IAM User and IAM Policy resources changed from read-only (Data) to managed (Managed+Elevated) — they can now be created, updated, and destroyed through the standard syncer interface
- New `CredentialScope` enum (Elevated/Regular) on `ResourceSpec` enables two-provider execution: admin credentials for IAM resources, scoped credentials for everything else
- Single unified Provision page replaces CredentialIntake, ScanProvision, and AwsManage pages
- Manifest version bumped to 7

### Fixed
- Translation now works even when no preferred chat model is set. `maybe_translate` previously read `cfg.preferred_model_id` and silently no-opped with a log warning when it was `None`, so enabling Translate without first picking a chat model produced empty translations with no UI signal. Replaced with a pinned `TRANSLATION_MODEL_ID = "us.anthropic.claude-sonnet-4-6"` constant (mirrors the `EXTRACTION_MODEL_ID` pattern). Sonnet 4.6 chosen for clinical-vocab handling; absolute cost (~$0.018/session) is a rounding error against Transcribe spend
- Drift card for array-valued fields (e.g. IAM policy `Actions`) now renders a single unified list with `+ added` / `- removed` / unchanged rows instead of striking out the entire expected array. Localized to `FieldDriftList.tsx`; falls back to the old two-block layout for scalar or object-shaped drift
- Eye-icon preview on audio files now opens the structured TranscriptEditor (speaker rename, segment editing) instead of falling through to the read-only `<pre>` view. Gate was matching on a `.text` suffix that the audio filename doesn't carry
- Version-history modal on audio files now lists transcript revisions (the `.text` sidecar) instead of the immutable audio object's one-entry history; modal header reads "Transcript History" for audio files. Non-audio files unaffected
- Drag-drop onto the records page no longer fires twice under React 18 StrictMode — the listener-registration Promise sometimes resolved after the first cleanup, leaving two live listeners. Added a `cancelled` flag that drains the listener if cleanup runs before registration completes
- Drag-drop onto the open TranscribeWizard modal is now routed into the wizard's file slot (with a "Drop file to use it" hover state) instead of bypassing the wizard's per-file options and uploading via the legacy single-language path
- Delete-file confirmation modal no longer says "This cannot be undone" — deleted files land in the deleted-files list and can be restored via the More-mode UI (S3 versioning keeps the delete marker as a regular version)
- Replaced custom `build.rs` JS build with Tauri's built-in `beforeBuildCommand`/`beforeDevCommand`, enabling Vite dev server hot-reload during development
- `claria-desktop` build was broken on `main` — `commands.rs` referenced the removed `manifest_version` field on `ProvisionerState` and used `aws_sdk_iam` without declaring it as a dependency. `claria_provisioner::create_access_key` now takes `&SdkConfig` instead of `&aws_sdk_iam::Client`, matching the library-crate boundary rule
- `cargo run -p claria-desktop` now reinstalls and rebuilds the frontend when `claria-desktop-frontend/package-lock.json` is newer than `dist/index.html`. Plain `cargo run` previously skipped `tauri.conf.json`'s `beforeBuildCommand`, leaving the bundled `@tauri-apps/api` JS at a stale version after a Tauri bump and crashing the WebView on launch (`web content process terminated`). Set `CLARIA_SKIP_FRONTEND_BUILD=1` or `CI=true` to opt out
- CI now smoke-tests the built `claria-desktop` binary under `xvfb` and fails the build if the WebView terminates within 15 s of launch — catches Rust/JS Tauri API version drift before it lands on `main`
- CI Tauri-build job sped up from ~5–6 m → ~3 m end-to-end: dropped the `needs: [frontend, clippy, test]` gate so `tauri-build` runs in parallel; moved `claria-desktop` clippy/test into the workspace Clippy/Test jobs (no more inline run in the build job); switched the release link step to the `mold` linker; cached the apt deps (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, plus `xvfb` on the build job) via `awalsh128/cache-apt-pkgs-action`; and skipped the 15 s xvfb smoke test on draft PRs.

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

# Changelog

All notable changes to Claria are documented here.

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

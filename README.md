# Claria

Self-hosted psychological report management platform. Deploys entirely into your own AWS account — your data never leaves infrastructure you control.

A desktop app (Tauri) walks you through AWS setup, provisions the backend, and manages the system lifecycle. The backend runs as a Lambda behind API Gateway, with S3 for storage, Cognito for auth, Bedrock for AI-assisted report drafting, and Tantivy for full-text search.

## Architecture

```
claria-desktop (Tauri)
  └─ claria-provisioner        # Creates/updates/destroys the AWS stack
  └─ claria-desktop-frontend   # React + TypeScript + Tailwind UI

claria-lambda (Lambda + API Gateway)
  ├─ claria-core               # Domain types, S3 key layout, Tantivy schema
  ├─ claria-storage            # S3 read/write operations
  ├─ claria-search             # Tantivy full-text index lifecycle
  ├─ claria-instruments        # Clinical assessment instruments (8 built-in)
  ├─ claria-bedrock            # Bedrock model invocation for report drafting
  ├─ claria-auth               # Cognito authentication
  ├─ claria-audit              # CloudTrail event helpers
  └─ claria-export             # Template rendering and DOCX generation
```

## Status

Early development. The workspace compiles and the desktop app boots, but provisioner integration is not yet wired up.

### What's built

- [x] Cargo workspace with 11 crates
- [x] `claria-core` — domain types (Patient, Report, ClinicalNote, etc.), Tantivy schema, S3 key layout, ts-rs bindings
- [x] `claria-instruments` — 8 clinical assessment instruments with scoring logic
- [x] `claria-storage` — S3 get/put/list/delete operations
- [x] `claria-search` — Tantivy index create/open/add/search/delete
- [x] `claria-bedrock` — Claude model invocation via Bedrock
- [x] `claria-auth` — Cognito sign-up/sign-in/token refresh/password flows
- [x] `claria-audit` — CloudTrail event parsing helpers
- [x] `claria-export` — Tera template rendering and DOCX generation
- [x] `claria-lambda` — Axum routes and middleware for the API
- [x] `claria-provisioner` — AWS resource lifecycle management (scaffold)
- [x] `claria-desktop` — Tauri app with config persistence, credential handling, STS validation
- [x] `claria-desktop-frontend` — React wizard flow (AWS guide, IAM guide, credential intake) and dashboard

### What's next

- [ ] Provisioner: scan, plan, execute, destroy lifecycle
- [ ] Provisioner: S3 state persistence with local backup
- [ ] Desktop: wire provisioner commands into the UI
- [ ] Desktop: scan/provision step in the wizard
- [ ] Desktop: manage dashboard with resource status
- [ ] Lambda: end-to-end API testing
- [ ] Export: report template library
- [ ] Search: index sync with S3 on Lambda cold start

## Development

Requires Rust (2024 edition), Node.js, and npm.

```sh
# Check everything compiles
cargo check

# Build and launch the desktop app (auto-builds the frontend)
cargo run -p claria-desktop
```

## License

UNLICENSED — proprietary.

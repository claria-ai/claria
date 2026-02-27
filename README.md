# Claria

Self-hosted psychological report management platform. Deploys entirely into your own AWS account — your data never leaves infrastructure you control.

A desktop app (Tauri) talks directly to AWS services — no Lambda, no API Gateway, no server to maintain. It walks you through AWS setup, provisions and hardens your S3 bucket, and handles case management, report generation, search, and export all from the desktop.

## Architecture

```
claria-desktop (Tauri — the whole product)
  ├─ claria-provisioner        # S3 hardening, CloudTrail, Bedrock access verification
  ├─ claria-storage            # S3 read/write operations
  ├─ claria-search             # Local Tantivy full-text index, backed up to S3
  ├─ claria-instruments        # Clinical assessment instruments (8 built-in)
  ├─ claria-bedrock            # Bedrock model invocation for report drafting
  ├─ claria-audit              # S3 audit trail + CloudTrail verification (HIPAA)
  ├─ claria-export             # Template rendering and DOCX generation
  ├─ claria-core               # Domain types, S3 key layout, Tantivy schema
  └─ claria-desktop-frontend   # React + TypeScript + Tailwind UI
```

## Status

Early development. The workspace compiles and the desktop app boots with a setup wizard, but provisioner and case management are not yet wired up.

### What's built

- [x] Cargo workspace with 9 crates
- [x] `claria-core` — domain types (Patient, Report, ClinicalNote, etc.), Tantivy schema, S3 key layout, ts-rs bindings
- [x] `claria-instruments` — 8 clinical assessment instruments with scoring logic
- [x] `claria-storage` — S3 get/put/list/delete operations
- [x] `claria-search` — Tantivy index create/open/add/search/delete
- [x] `claria-bedrock` — Claude model invocation via Bedrock
- [x] `claria-audit` — CloudTrail event helpers (being rewritten for S3 audit trail)
- [x] `claria-export` — Tera template rendering and DOCX generation
- [x] `claria-provisioner` — AWS resource lifecycle management (scaffold, being simplified)
- [x] `claria-desktop` — Tauri app with config persistence, credential handling, STS validation
- [x] `claria-desktop-frontend` — React wizard flow (AWS guide, IAM guide, credential intake) and dashboard

### What's next

- [ ] Provisioner: simplify to S3 + CloudTrail + Bedrock (remove Lambda/Gateway/Cognito resources)
- [ ] Provisioner: S3 bucket security hardening (encryption, versioning, public access block)
- [ ] Provisioner: scan, plan, execute with local + S3 state persistence
- [ ] Audit: rewrite for S3-based audit trail + CloudTrail verification
- [ ] Desktop: wire provisioner commands into the UI
- [ ] Desktop: case management (assessments, reports, templates)
- [ ] Desktop: Bedrock-powered report generation
- [ ] Desktop: local Tantivy search with S3 backup
- [ ] Export: DOCX/PDF generation from templates

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

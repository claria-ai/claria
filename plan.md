# Mock AWS Service — Implementation Plan

## Goal

A standalone Rust binary (`claria-mock-aws`) that emulates the AWS APIs Claria uses, enabling serial E2E tests with the real Tauri app + Playwright. No LocalStack, no Docker — just a single `cargo run` binary.

## Architecture

```
Playwright (serial, workers: 1)
    │
    ▼
cargo tauri dev  ──→  claria-mock-aws (axum, :9000)
    │                      │
    ├── claria-storage ────┤  S3 API
    ├── claria-provisioner─┤  S3 + IAM + STS + CloudTrail + Bedrock + Artifact
    ├── claria-bedrock ────┤  Bedrock + Bedrock Runtime
    ├── claria-transcribe──┤  Transcribe + S3
    └── claria-billing ────┤  Cost Explorer
```

Single HTTP server, single port. AWS SDK clients all get `endpoint_url("http://localhost:9000")`.

## Service Routing

All AWS services on one port, distinguished by request shape:

| Signal | Service |
|--------|---------|
| `X-Amz-Target: com.amazonaws.cloudtrail.v20131101.CloudTrail_20131101.*` | CloudTrail |
| `X-Amz-Target: Transcribe.*` | Transcribe |
| `X-Amz-Target: AWSInsightsIndexService.*` | Cost Explorer |
| `X-Amz-Target: artifact.*` | Artifact |
| Path starts with `/foundation-models` or `/custom-model-agreements` or `/inference-profiles` or `/logging/model-invocations` | Bedrock |
| Path starts with `/model/` (Converse) | Bedrock Runtime |
| Query param `Action=` present | IAM or STS (distinguished by action name) |
| Everything else | S3 (path-style: `/{bucket}` or `/{bucket}/{key...}`) |

## In-Memory State

```rust
struct MockState {
    // S3
    buckets: HashMap<String, BucketState>,       // bucket name → metadata + versioning config
    objects: HashMap<(String, String), Vec<ObjectVersion>>,  // (bucket, key) → version stack
    bucket_policies: HashMap<String, Value>,
    bucket_encryption: HashMap<String, String>,  // bucket → algorithm
    bucket_public_access: HashMap<String, PublicAccessBlock>,

    // IAM
    users: HashMap<String, IamUser>,
    policies: HashMap<String, IamPolicy>,        // policy ARN → document + versions
    user_attached_policies: HashMap<String, Vec<String>>,  // user → policy ARNs
    user_inline_policies: HashMap<(String, String), String>,  // (user, policy_name) → document
    access_keys: HashMap<String, AccessKeyRecord>,  // access_key_id → metadata

    // STS
    caller_identity: CallerIdentity,             // configurable per-scenario

    // CloudTrail
    trails: HashMap<String, Trail>,
    trail_logging: HashMap<String, bool>,

    // Bedrock
    foundation_models: Vec<FoundationModel>,     // pre-loaded model catalog
    model_agreements: HashSet<String>,            // accepted model IDs
    inference_profiles: Vec<InferenceProfile>,

    // Transcribe
    transcription_jobs: HashMap<String, TranscriptionJob>,

    // Cost Explorer
    cost_data: Vec<CostPeriod>,                  // pre-loaded fixture data

    // Artifact
    baa_accepted: bool,
}
```

### Reset & Scenarios

```
POST /mock/reset              → clear all state to empty
POST /mock/scenario/{name}    → load a named preset
```

Preset scenarios:
- `fresh-account` — STS identity exists, nothing else provisioned
- `bootstrapped` — IAM user + policy created, no infrastructure
- `fully-provisioned` — all 14 resources in sync, sample clients/files in S3
- `drifted` — versioning disabled, encryption missing (tests drift detection)

## Phase 1: Crate Skeleton + S3 Core + Endpoint Plumbing

### 1a. Create `claria-mock-aws` crate

New workspace member at `crates/claria-mock-aws/`:
- `Cargo.toml` — axum, tokio, serde, serde_json, uuid
- `src/main.rs` — CLI (port flag), starts axum server
- `src/state.rs` — `MockState` with `Arc<RwLock<_>>`
- `src/router.rs` — top-level router with service dispatch
- `src/services/mod.rs` — service modules

### 1b. S3 API

Priority operations (used by claria-storage + provisioner + transcribe):

| Method + Path | AWS Operation | Response Format |
|---------------|---------------|-----------------|
| `HEAD /{bucket}` | HeadBucket | 200 or 404 |
| `PUT /{bucket}` | CreateBucket | 200 XML |
| `DELETE /{bucket}` | DeleteBucket | 204 |
| `GET /{bucket}?versioning` | GetBucketVersioning | XML `<VersioningConfiguration>` |
| `PUT /{bucket}?versioning` | PutBucketVersioning | 200 |
| `GET /{bucket}?encryption` | GetBucketEncryption | XML |
| `PUT /{bucket}?encryption` | PutBucketEncryption | 200 |
| `GET /{bucket}?publicAccessBlock` | GetPublicAccessBlock | XML |
| `PUT /{bucket}?publicAccessBlock` | PutPublicAccessBlock | 200 |
| `DELETE /{bucket}?publicAccessBlock` | DeletePublicAccessBlock | 204 |
| `GET /{bucket}?policy` | GetBucketPolicy | JSON string body |
| `PUT /{bucket}?policy` | PutBucketPolicy | 200 |
| `DELETE /{bucket}?policy` | DeleteBucketPolicy | 204 |
| `GET /{bucket}?list-type=2` | ListObjectsV2 | XML `<ListBucketResult>` |
| `GET /{bucket}?versions` | ListObjectVersions | XML |
| `HEAD /{bucket}/{key..}` | HeadObject | 200 with headers or 404 |
| `GET /{bucket}/{key..}` | GetObject | Body + headers (ETag, Content-Type) |
| `GET /{bucket}/{key..}?versionId=X` | GetObject (versioned) | Specific version body |
| `PUT /{bucket}/{key..}` | PutObject | 200, generate ETag + version ID |
| `DELETE /{bucket}/{key..}` | DeleteObject | 204, insert delete marker if versioned |

S3 versioning behavior:
- When versioning enabled: PutObject appends new version, DeleteObject inserts delete marker
- Version IDs are UUIDs
- ListObjectVersions returns all versions + delete markers sorted by key then timestamp

### 1c. Endpoint URL support in `build_aws_config`

Modify `crates/claria-desktop/src/aws.rs`:

```rust
pub async fn build_aws_config(
    region: &str,
    creds: &CredentialSource,
    endpoint_url: Option<&str>,
) -> aws_config::SdkConfig {
    let mut builder = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region.to_string()));

    if let Some(url) = endpoint_url {
        builder = builder.endpoint_url(url);
    }

    // ... credential setup unchanged ...

    builder.load().await
}
```

The endpoint_url is read from `CLARIA_AWS_ENDPOINT` env var in `load_sdk_config()` in `commands.rs`. When set, all AWS clients point at the mock.

S3 needs `force_path_style(true)` when using custom endpoints. This is set per-client in each crate that builds an S3 client, conditioned on whether the endpoint URL is non-default. The `SdkConfig` doesn't have a global path-style toggle — it's an S3-specific client config. Two options:

**Option A:** Each crate that builds `aws_sdk_s3::Client::new(config)` checks `config.endpoint_url()` and if set, uses `Client::from_conf(config_builder.force_path_style(true).build())` instead. This is ~4 call sites (storage, provisioner, transcribe, desktop).

**Option B:** Add a wrapper in `claria-core`:
```rust
pub fn s3_client(config: &SdkConfig) -> aws_sdk_s3::Client {
    let mut builder = aws_sdk_s3::config::Builder::from(config);
    if config.endpoint_url().is_some() {
        builder = builder.force_path_style(true);
    }
    aws_sdk_s3::Client::from_conf(builder.build())
}
```

**Recommend Option B** — single call site, consistent behavior.

## Phase 2: STS + IAM

### STS

| Action | Response |
|--------|----------|
| `GetCallerIdentity` | XML: `<Account>`, `<Arn>`, `<UserId>` from `MockState.caller_identity` |
| `AssumeRole` | XML: temporary credentials (mock key/secret/token + expiration) |

### IAM

| Action | Response |
|--------|----------|
| `GetUser` | XML: user ARN, user ID, create date |
| `CreateUser` | XML: user ARN |
| `ListUsers` | XML: user list (paginated) |
| `CreatePolicy` | XML: policy ARN, version ID |
| `GetPolicy` | XML: policy ARN, default version ID |
| `GetPolicyVersion` | XML: URL-encoded policy document |
| `CreatePolicyVersion` | XML: version ID (delete oldest if at 5) |
| `ListPolicyVersions` | XML: version list |
| `DeletePolicyVersion` | 200 |
| `AttachUserPolicy` | 200 |
| `DetachUserPolicy` | 200 |
| `ListAttachedUserPolicies` | XML: list of policy name + ARN |
| `GetUserPolicy` | XML: policy document |
| `PutUserPolicy` | 200 |
| `DeleteUserPolicy` | 200 |
| `CreateAccessKey` | XML: access key ID + secret |
| `ListAccessKeys` | XML: key metadata list |
| `DeleteAccessKey` | 200 |
| `GetAccessKeyLastUsed` | XML: last used date + service name |

IAM and STS both use AWS Query protocol — `POST` with `Action=X` in form body, XML responses.

## Phase 3: CloudTrail + Bedrock + Bedrock Runtime

### CloudTrail (JSON protocol, `X-Amz-Target` header)

| Target suffix | Response |
|---------------|----------|
| `CreateTrail` | JSON: trail ARN, name, S3 bucket |
| `GetTrail` | JSON: trail object |
| `DeleteTrail` | 200 |
| `StartLogging` | 200 |
| `StopLogging` | 200 |
| `GetTrailStatus` | JSON: `{ "IsLogging": bool }` |

### Bedrock (REST, JSON bodies)

| Method + Path | Operation | Response |
|---------------|-----------|----------|
| `GET /foundation-models?byProvider=anthropic` | ListFoundationModels | JSON: model summaries |
| `GET /inference-profiles?typeEquals=SYSTEM_DEFINED` | ListInferenceProfiles | JSON: profile list |
| `GET /foundation-models/{id}/availability` | GetFoundationModelAvailability | JSON |
| `GET /foundation-models/{id}/agreement-offers` | ListFoundationModelAgreementOffers | JSON: offer tokens |
| `POST /custom-model-agreements` | CreateFoundationModelAgreement | JSON: agreement ID |

### Bedrock Runtime (REST, JSON bodies)

| Method + Path | Operation | Response |
|---------------|-----------|----------|
| `POST /model/{id}/converse` | Converse | JSON: canned response with usage stats |

The Converse response returns a canned message. For E2E tests, the content doesn't need to be dynamic — a fixed response per model is fine. The response format:

```json
{
  "output": {
    "message": {
      "role": "assistant",
      "content": [{ "text": "..." }]
    }
  },
  "stopReason": "end_turn",
  "usage": { "inputTokens": 150, "outputTokens": 200 }
}
```

## Phase 4: Transcribe + Cost Explorer + Artifact

### Transcribe (JSON protocol)

| Target suffix | Response |
|---------------|----------|
| `StartTranscriptionJob` | JSON: job with status `IN_PROGRESS` |
| `GetTranscriptionJob` | JSON: job with status `COMPLETED` + transcript URI |
| `DeleteTranscriptionJob` | 200 |

The mock auto-completes jobs immediately (no polling delay needed). When `GetTranscriptionJob` is called, it returns `COMPLETED` and writes the transcript output to the mock S3 state at `_transcribe/{job_name}.json`.

### Cost Explorer (JSON protocol)

| Target suffix | Response |
|---------------|----------|
| `GetCostAndUsage` | JSON: `ResultsByTime` with pre-loaded fixture data |

### Artifact (JSON protocol)

| Target suffix | Response |
|---------------|----------|
| `ListCustomerAgreements` | JSON: agreement list (BAA active or empty) |

## Phase 5: E2E Test Infrastructure

### Playwright config (`e2e/playwright.config.ts`)

```typescript
export default defineConfig({
  testDir: ".",
  timeout: 120_000,
  workers: 1,   // serial — one Tauri instance
  use: {
    viewport: { width: 1024, height: 768 },
    deviceScaleFactor: 2,
  },
  webServer: [
    {
      command: "cargo run -p claria-mock-aws -- --port 9000",
      url: "http://localhost:9000/mock/health",
      reuseExistingServer: true,
      timeout: 60_000,
    },
    {
      command: "CLARIA_AWS_ENDPOINT=http://localhost:9000 cargo tauri dev",
      url: "http://localhost:1420",
      reuseExistingServer: true,
      timeout: 120_000,
    },
  ],
});
```

### Test lifecycle

```typescript
test.beforeEach(async ({ request }) => {
  // Reset mock state before each test
  await request.post("http://localhost:9000/mock/reset");
});

test("onboarding flow", async ({ page, request }) => {
  // Load fresh-account scenario
  await request.post("http://localhost:9000/mock/scenario/fresh-account");
  await page.goto("http://localhost:1420");
  // ... drive the UI through onboarding
});
```

### Test scenarios (initial set)

1. **Onboarding** — fresh account → enter credentials → bootstrap IAM user → provision infrastructure
2. **Fully provisioned dashboard** — load pre-provisioned state, verify all resources show green
3. **Client management** — create client, upload file, extract text, delete client
4. **Chat** — open client record, send message, verify response renders
5. **Drift detection** — load drifted scenario, verify plan shows corrective actions
6. **Cost explorer** — verify cost chart renders with fixture data
7. **File versioning** — upload, modify, view history, restore old version

## Implementation Order

| Step | What | Depends on |
|------|------|------------|
| 1 | Crate skeleton + axum router + state + reset endpoint | — |
| 2 | S3 core (CRUD, list, head) | 1 |
| 3 | `s3_client()` helper in claria-core + endpoint_url in aws.rs | 1 |
| 4 | STS (GetCallerIdentity, AssumeRole) | 1 |
| 5 | S3 bucket operations (versioning, encryption, public access, policy) | 2 |
| 6 | IAM (users, policies, access keys) | 1 |
| 7 | CloudTrail | 1 |
| 8 | Bedrock + Bedrock Runtime | 1 |
| 9 | Transcribe | 2 |
| 10 | Cost Explorer + Artifact | 1 |
| 11 | Scenario presets | 2–10 |
| 12 | Playwright E2E scaffold + first test | 3, 11 |
| 13 | Full test suite | 12 |

Steps 4–10 are largely independent and can be built in any order after step 1.

## What This Does NOT Cover

- **Presigned URLs** — the mock could generate them but they'd point back at the mock server. If the frontend uses presigned URLs for downloads, those will work since the mock S3 handles GET requests. Presign generation in tests may need the mock to return `http://localhost:9000/{bucket}/{key}` style URLs.
- **Streaming responses** — Bedrock Runtime `Converse` returns non-streaming JSON. If the app uses `ConverseStream`, that would need SSE/chunked encoding. Current code uses non-streaming `Converse`.
- **Multi-region** — all requests go to one endpoint regardless of region.
- **Auth signature validation** — the mock ignores SigV4 signatures entirely. Any credentials work.
- **Rate limiting / throttling** — not simulated.
- **Error injection** — could be added later via `POST /mock/fault/{service}` to test error handling paths.

## File Layout

```
crates/claria-mock-aws/
├── Cargo.toml
├── src/
│   ├── main.rs            # CLI + server startup
│   ├── state.rs           # MockState + Arc<RwLock<_>>
│   ├── router.rs          # Top-level axum router + service dispatch
│   ├── scenarios.rs       # Preset scenario loaders
│   ├── xml.rs             # XML response helpers (for S3/IAM/STS)
│   └── services/
│       ├── mod.rs
│       ├── s3.rs           # S3 API handlers
│       ├── iam.rs          # IAM API handlers
│       ├── sts.rs          # STS API handlers
│       ├── cloudtrail.rs   # CloudTrail handlers
│       ├── bedrock.rs      # Bedrock + Runtime handlers
│       ├── transcribe.rs   # Transcribe handlers
│       ├── cost_explorer.rs # Cost Explorer handlers
│       └── artifact.rs     # Artifact handlers
e2e/
├── playwright.config.ts
├── package.json
├── tsconfig.json
└── tests/
    ├── onboarding.spec.ts
    ├── dashboard.spec.ts
    ├── clients.spec.ts
    ├── chat.spec.ts
    └── ...
```

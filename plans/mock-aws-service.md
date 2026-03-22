# Plan: `claria-mock-aws` — Fake AWS Service for E2E Testing

## Overview

A new workspace crate that runs an HTTP server emulating the AWS APIs Claria uses.
Each test gets an ephemeral "account" (isolated namespace keyed by access key ID),
enabling concurrent, stateful E2E tests with no real AWS credentials.

## Architecture

```
┌─────────────────────────────────────────────┐
│              claria-mock-aws                │
│                                             │
│  ┌─────────┐  ┌──────────┐  ┌───────────┐  │
│  │  Axum   │  │ Service  │  │   State   │  │
│  │ Router  │→ │ Dispatch │→ │   Store   │  │
│  └─────────┘  └──────────┘  └───────────┘  │
│       ↑                          │          │
│   HTTP req                  ┌────┴────┐     │
│   (AWS SDK)                 │ SQLite  │     │
│                             │ + files │     │
│  ┌──────────────────┐       └─────────┘     │
│  │ Admin API        │                       │
│  │ POST /_mock/...  │                       │
│  └──────────────────┘                       │
└─────────────────────────────────────────────┘
```

### Request Routing

The AWS SDK sends requests with service-identifying headers/paths:

| Service | Discrimination | Format |
|---------|---------------|--------|
| **S3** | Path-based: `/{bucket}` or `/{bucket}/{key}` | XML request/response |
| **STS** | `POST /` with `Action=GetCallerIdentity` form body | XML response |
| **IAM** | `POST /` with `Action=CreateUser` etc. form body | XML response |
| **CloudTrail** | `x-amz-target: CloudTrail_20131101.GetTrail` | JSON request/response |
| **Bedrock** | Path: `/foundation-models`, `/model-agreements` etc. | JSON request/response |
| **Bedrock Runtime** | Path: `/model/{model_id}/converse` | JSON request/response |
| **Transcribe** | `x-amz-target: Transcribe.StartTranscriptionJob` | JSON request/response |
| **Cost Explorer** | `x-amz-target: AWSInsightsIndexService.GetCostAndUsage` | JSON request/response |
| **Artifact** | Path: `/customer-agreements` | JSON request/response |

**Problem**: The AWS SDK sends each service to a different hostname (e.g.,
`s3.us-east-1.amazonaws.com`, `iam.amazonaws.com`). With a single mock server,
we use `endpoint_url("http://localhost:{port}")` on the `SdkConfig`, which makes
the SDK send ALL services to the same host. We then discriminate by:

1. **Path prefix** (S3 uses `/{bucket}/{key}`, Bedrock uses `/foundation-models`)
2. **`x-amz-target` header** (CloudTrail, Transcribe, Cost Explorer)
3. **`Action` form parameter** (IAM, STS — both use `POST /` with form-encoded body)

Since IAM and STS both POST to `/` with form params, we differentiate by the
`Action` value (IAM actions: `CreateUser`, `GetUser`, etc.; STS actions:
`GetCallerIdentity`, `AssumeRole`).

### Account Isolation

Every request carries AWS credentials via the `Authorization` header (SigV4).
We parse the **access key ID** from the `Credential=` component — this becomes
the account namespace key. No signature verification needed.

```
Authorization: AWS4-HMAC-SHA256 Credential=AKID.../20260322/us-east-1/s3/aws4_request, ...
                                            ^^^^
                                         extract this
```

### State Store

**SQLite** (via `rusqlite`) for structured data, **filesystem** for S3 object bodies.

```
{data_dir}/
├── mock.db          # SQLite: IAM, STS, CloudTrail, Transcribe, Bedrock metadata
└── s3/
    └── {access_key}/
        └── {bucket}/
            └── {key}         # Raw object bytes
```

SQLite tables (all keyed by `account_id` = access key):

- `accounts` — access_key, secret_key, account_number, arn
- `s3_objects` — account_id, bucket, key, etag, content_type, size, version_id, is_deleted, created_at
- `s3_buckets` — account_id, bucket, versioning_status, encryption_config, public_access_block, policy
- `iam_users` — account_id, user_name, arn, created_at
- `iam_policies` — account_id, policy_name, policy_arn, document (JSON), version_id, is_default
- `iam_user_policy_attachments` — account_id, user_name, policy_arn
- `iam_access_keys` — account_id, user_name, access_key_id, secret_key, status, created_at
- `cloudtrail_trails` — account_id, trail_name, s3_bucket, s3_prefix, is_logging, is_multi_region
- `transcribe_jobs` — account_id, job_name, status, media_uri, output_key, failure_reason, created_at
- `bedrock_model_agreements` — account_id, model_id, offer_token, status

### Admin API (non-AWS endpoints)

| Endpoint | Purpose |
|----------|---------|
| `POST /_mock/accounts` | Create ephemeral account → returns `{ access_key, secret_key, account_id }` |
| `DELETE /_mock/accounts/{access_key}` | Tear down account + all its state |
| `POST /_mock/accounts/{access_key}/transcribe/{job_name}/complete` | Force-complete a transcribe job (since there's no real audio processing) |
| `GET /_mock/health` | Health check |

### Test Helper: `mock_sdk_config()`

A public function that:
1. Calls `POST /_mock/accounts` to create an account
2. Builds an `aws_config::SdkConfig` with:
   - `endpoint_url("http://localhost:{port}")`
   - The returned credentials
   - `region("us-east-1")`
3. Returns `(SdkConfig, MockAccount)` where `MockAccount` has a `Drop` impl
   that calls `DELETE /_mock/accounts/{access_key}`

---

## Service Implementations (by priority)

### Phase 1: S3 + STS (unlocks claria-storage, claria-search tests)

**STS operations:**
- `GetCallerIdentity` → return the account's canned identity

**S3 operations:**
- `PutObject` — store body to disk, metadata to SQLite, generate ETag (MD5 hex)
- `PutObject` with `If-Match` — compare ETag, return `412 PreconditionFailed` on mismatch
- `GetObject` — return body + ETag + content-type
- `GetObject` with `versionId` — return specific version
- `DeleteObject` — soft-delete (mark in DB), remove body if not versioned
- `ListObjectsV2` — filter by prefix, paginate with continuation token
- `ListObjectVersions` — return versions + delete markers
- `CreateBucket` — create entry in `s3_buckets`
- `DeleteBucket` — remove if empty
- `HeadBucket` — check existence
- Bucket config operations: Get/Put Versioning, Encryption, PublicAccessBlock, Policy
- `PresignedGet` / `PresignedPut` — presigning is client-side, but the resulting PUT/GET
  must work against the mock. No special handling needed — the regular Put/Get handlers
  serve presigned requests too.

### Phase 2: IAM (unlocks claria-provisioner tests)

**Operations:**
- `CreateUser` / `GetUser` / `ListUsers`
- `CreatePolicy` / `GetPolicy` / `GetPolicyVersion` / `CreatePolicyVersion` / `ListPolicyVersions` / `DeletePolicyVersion`
- `AttachUserPolicy` / `ListAttachedUserPolicies`
- `CreateAccessKey` / `ListAccessKeys` / `GetAccessKeyLastUsed` / `DeleteAccessKey`

**Error simulation:**
- `EntityAlreadyExistsException` on duplicate create
- `NoSuchEntityException` on missing get
- `LimitExceededException` when >5 policy versions

### Phase 3: CloudTrail (unlocks audit + provisioner trail tests)

**Operations:**
- `GetTrail` / `CreateTrail` / `DeleteTrail`
- `GetTrailStatus` / `StartLogging` / `StopLogging`

### Phase 4: Bedrock + Bedrock Runtime (unlocks claria-bedrock tests)

**Bedrock operations:**
- `ListFoundationModels` — return a hardcoded list of Anthropic Claude models
- `ListInferenceProfiles` — return `us.*` profiles for each model
- `GetFoundationModelAvailability` → `Available`
- `ListFoundationModelAgreementOffers` → canned offer token
- `CreateFoundationModelAgreement` → store in DB

**Bedrock Runtime operations:**
- `Converse` — return a canned response with configurable text
  (admin API could allow setting response text per model, or use a deterministic
  echo like "Mock response to: {first N chars of input}")
- `CountTokens` — return `input_tokens = word_count * 1.3` (rough approximation)

### Phase 5: Transcribe (unlocks claria-transcribe tests)

**Operations:**
- `StartTranscriptionJob` — store job in DB with status `IN_PROGRESS`
- `GetTranscriptionJob` — return current status
- `DeleteTranscriptionJob` — remove from DB

**Completion flow:** Jobs don't auto-complete (no real audio processing).
The admin API `POST /_mock/accounts/{ak}/transcribe/{job}/complete` sets status
to `COMPLETED` and writes a canned transcript JSON to the S3 output key.

### Phase 6: Cost Explorer + Artifact (unlocks claria-billing, BAA checks)

**Cost Explorer:**
- `GetCostAndUsage` — return canned cost data with $0.00 amounts

**Artifact:**
- `ListCustomerAgreements` — return empty list (no BAA) by default;
  admin API to seed an active BAA

---

## Crate Structure

```
crates/claria-mock-aws/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API: MockServer, mock_sdk_config()
│   ├── server.rs           # Axum server setup, router, middleware
│   ├── dispatch.rs         # Route to correct service handler based on headers/path
│   ├── auth.rs             # Extract access_key from Authorization header
│   ├── store.rs            # SQLite + filesystem state store
│   ├── admin.rs            # /_mock/* admin endpoints
│   ├── services/
│   │   ├── mod.rs
│   │   ├── s3.rs           # S3 API handlers
│   │   ├── sts.rs          # STS handlers
│   │   ├── iam.rs          # IAM handlers
│   │   ├── cloudtrail.rs   # CloudTrail handlers
│   │   ├── bedrock.rs      # Bedrock + Bedrock Runtime handlers
│   │   ├── transcribe.rs   # Transcribe handlers
│   │   ├── cost_explorer.rs
│   │   └── artifact.rs
│   └── xml.rs              # XML response helpers (S3, IAM, STS use XML)
└── tests/
    ├── s3_test.rs
    ├── iam_test.rs
    └── integration_test.rs
```

### Dependencies

```toml
[package]
name = "claria-mock-aws"
version = "0.1.0"
edition.workspace = true

[dependencies]
axum = "=0.8.4"
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true }
serde_json = { workspace = true }
rusqlite = { version = "=0.35.0", features = ["bundled"] }
uuid = { version = "=1.16.0", features = ["v4"] }
md-5 = "=0.10.6"
hex = "=0.4.3"
quick-xml = "=0.37.5"           # For S3/IAM/STS XML responses
percent-encoding = "=2.3.1"     # IAM policy document encoding
aws-config = { workspace = true }
aws-credential-types = "=1.2.4"
thiserror = { workspace = true }
tracing = "=0.1.41"
```

### Public API

```rust
/// Start the mock server on a random available port.
pub async fn start() -> Result<MockServer, MockAwsError> { ... }

/// The running mock server handle.
pub struct MockServer {
    port: u16,
    // ...
}

impl MockServer {
    /// Base URL: http://localhost:{port}
    pub fn endpoint_url(&self) -> String { ... }

    /// Create an isolated ephemeral account and return an SdkConfig pointing at this server.
    pub async fn create_account(&self) -> Result<MockAccount, MockAwsError> { ... }

    /// Manually complete a transcribe job (writes canned transcript to S3).
    pub async fn complete_transcribe_job(
        &self, access_key: &str, job_name: &str, transcript: &str,
    ) -> Result<(), MockAwsError> { ... }

    /// Shut down the server.
    pub async fn shutdown(self) { ... }
}

/// An ephemeral AWS account with auto-cleanup on drop.
pub struct MockAccount {
    pub sdk_config: aws_config::SdkConfig,
    pub access_key: String,
    pub secret_key: String,
    pub account_id: String,
    // ...
}
```

### Usage in Tests

```rust
use claria_mock_aws::start;
use claria_storage::objects;

#[tokio::test]
async fn test_put_and_get_object() {
    let server = start().await.unwrap();
    let account = server.create_account().await.unwrap();

    // Create bucket first (provisioner would do this)
    // ... or use admin API to pre-seed

    objects::put_object(&account.sdk_config, "my-bucket", "test.txt", b"hello", "text/plain").await.unwrap();
    let result = objects::get_object(&account.sdk_config, "my-bucket", "test.txt").await.unwrap();
    assert_eq!(result.body, b"hello");
}
```

---

## Implementation Order

1. **Scaffold crate** — Cargo.toml, workspace member, basic axum server, SQLite schema
2. **Auth extraction** — Parse access key from SigV4 `Authorization` header
3. **Admin API** — Account create/delete, health check
4. **S3 service** — Object CRUD, bucket operations, versioning, ETags
5. **STS service** — `GetCallerIdentity`
6. **Write claria-storage integration tests** against the mock
7. **IAM service** — User/policy/key CRUD with error simulation
8. **CloudTrail service** — Trail CRUD + logging status
9. **Write claria-provisioner integration tests** against the mock
10. **Bedrock service** — Model listing, agreements, Converse with canned responses
11. **Transcribe service** — Job lifecycle with admin completion trigger
12. **Cost Explorer + Artifact** — Canned responses
13. **Write remaining integration tests**

## Open Questions

- **Shared test server**: Should tests share a single `MockServer` instance (started
  once via `lazy_static` / `once_cell`) or start one per test? Shared is faster but
  needs careful port management. Account isolation makes sharing safe.
- **Persistence across restarts**: For now, ephemeral (tempdir). Could add option for
  a persistent data dir if needed for manual testing / desktop dev.
- **Binary target**: Add a `[[bin]]` target so `cargo run -p claria-mock-aws` starts
  the server standalone? Useful for manual desktop testing against the mock.

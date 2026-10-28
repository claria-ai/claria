# Writer dependency options

Adopted Writer-branch dependencies and validation receipts, 2026-10-27. Claria and
the fork's embedded package now use the coherent AWS cohort below, Tokio 1.53.3,
and compatible transport security fixes. These are local Writer changes; the
concurrent extraction/error-sidecar release checkout was not edited. No dependency
policy exceptions, merge, release, deployment, or live AWS validation are included.

## Adopted pins

Existing AWS features and `default-features = false` selections are preserved.
Claria and embedded share compatible `SdkConfig`, credential and Bedrock types.

| Dependency | Previous pin | Adopted pin |
| --- | --- | --- |
| aws-config | 1.8.18 | 1.12.0 |
| aws-credential-types | 1.2.14 | 1.3.0 |
| aws-sdk-bedrockruntime | 1.135.0 | 1.146.0 |
| aws-sdk-bedrock | 1.147.0 | 1.158.0 |
| aws-sdk-s3 | 1.137.0 | 1.149.0 |
| aws-sdk-sts | 1.107.0 | 1.117.0 |
| aws-sdk-artifact | 1.94.0 | 1.104.0 |
| aws-sdk-cloudtrail | 1.112.0 | 1.123.0 |
| aws-sdk-costexplorer | 1.120.0 | 1.130.0 |
| aws-sdk-iam | 1.113.0 | 1.126.0 |
| aws-sdk-transcribe | 1.108.0 | 1.120.0 |
| aws-smithy-eventstream | 0.60.21 | 0.61.4 |
| aws-smithy-types | 1.5.0 | 1.8.0 |
| aws-smithy-runtime-api | 1.12.3 | 1.17.0 |
| aws-smithy-http-client | 1.1.13 | 1.4.2 |
| tokio | 1.52.3 | 1.53.3 |
| lru | 0.18.0 | 0.18.2 |
| rustls | 0.23.41 | 0.23.46 |

Embedded directly pins config, BedrockRuntime, Smithy types and Tokio from this
table, including the same Tokio version in normal and dev dependencies. Claria's
S3 upgrade requires the compatible `lru` patch. The AWS cohort requires Rust
1.94.1, within both packages' declared 1.97 requirement. Supporting resolved AWS
versions include `aws-runtime` 1.10.0, `aws-smithy-runtime` 1.15.0 and SigV4 1.6.0.

The old SDK cohort unconditionally included HTTP 0.2/body 0.4 compatibility types,
even with the legacy network connector disabled. AWS made those types optional in
its [September 22 migration](https://github.com/awslabs/aws-sdk-rust/blob/main/CHANGELOG.md#september-22nd-2026),
implemented by [Smithy PR 4827](https://github.com/smithy-lang/smithy-rs/pull/4827).
The coordinated upgrade removes HTTP 0.2/body 0.4 from both lockfiles while retaining
the existing Hyper 1 / h2 0.4 / rustls 0.23 / AWS-LC transport.

## Advisory fixes and transport reachability

| Advisory | Published patched range | Adopted resolution |
| --- | --- | --- |
| [Tokio RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html) | `>=1.51.6, <1.52.0` or `>=1.53.3` | 1.53.3 in both workspaces |
| [h2 RUSTSEC-2026-0258](https://rustsec.org/advisories/RUSTSEC-2026-0258.html) | `>=0.4.16` | Claria lock 0.4.15 → 0.4.20 |
| [rustls RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html) | `>=0.23.45` | 0.23.46 |
| rustls [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html), [0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html), [0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html) | `>=0.23.46` | 0.23.46 |
| [rustls-webpki RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) | `>=0.103.16, <0.104.0-alpha.1` or `>=0.104.0-alpha.8` | Claria lock 0.103.13 → 0.103.16 |
| [ctutils RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html) | `>=0.4.4` | Claria lock 0.4.2 → 0.4.4 |

HTTP/TLS packages are reached through the Bedrock SDK's Smithy HTTP client.
`ctutils` is reached through BedrockRuntime → SigV4 → HMAC → digest. Package
reachability is not proof that every advisory's exploit conditions apply; for
example, the rustls `brotli` feature is not enabled.

Patched rustls and WebPKI require AWS-LC 1.18, so Claria's lock also moves
`aws-lc-rs` 1.17.1 → 1.18.2 and `aws-lc-sys` 0.42.0 → 0.46.0. These match the
embedded probe's provider versions. Resolved feature sets were compared before
and after and are unchanged. Tokio 1.54.0 was not selected: it is yanked and has
the separate [Interval race](https://rustsec.org/advisories/RUSTSEC-2026-0384.html),
fixed in `>=1.54.1`.

The Claria transport lock receipt is
`261f7e18f2ed8d3386469e9c323b4fa9d40cc20d62348ccb5d2e3e41b147a3e2` (SHA-256).
Exact updates, feature comparisons and primary sources are recorded in
`/tmp/writer-transport-patch-receipt.json`. The pre-transport AWS changes were
preserved as `/tmp/writer-aws-cohort-{claria,embedded}.patch`.

## Explicit profile contract

The owner approved strict explicit profile selection: a selected profile must
supply the identity, without top-level environment-credential override or fallback
to unrelated credential providers. Inline and DefaultChain modes retain their
separate contracts. This is an intentional narrowing of the former Profile mode.

The reason is concrete: AWS's
[July 8 change](https://github.com/awslabs/aws-sdk-rust/blob/main/CHANGELOG.md#july-8th-2026)
maps missing/token-only profiles to `CredentialsNotLoaded`, permitting the default
chain to continue. `BehaviorVersion` cannot restore the old unconditional error
mapping. The selected-profile implementation uses the SDK's native profile
provider with the existing shared client, explicit region, retries and timeouts.
A programmatic SigV4 preference also prevents ambient Bedrock bearer tokens from
bypassing profile selection. Inline and DefaultChain retain their existing bearer
behavior.

Thirteen actual-source tests passed (12 isolated subprocess scenarios and their
child entrypoint), with environment-cleared synthetic profile files and loopback
HTTP only. They cover explicit-profile precedence, missing/token-only profiles,
lazy credential processes, actual selected-profile Bedrock signing, a missing
profile producing zero HTTP requests, and unchanged DefaultChain/Inline behavior.
All-target Clippy passed; independent source review found no remaining blocker.
The isolated crate includes the actual desktop `aws.rs` and repository test file,
shimming only `CredentialSource`, because this environment lacks GTK/WebKit for a
full Tauri build. Live SSO, assume-role and AWS behavior remain unvalidated.
Receipts: `/tmp/writer-profile-contract/receipt.json` and
`/tmp/writer-profile-contract-{test,clippy}.log`.

## Recorded validation and remaining work

- **Active embedded harness:** 35 tests passed (1 unit, 19 agent, 13 collector, 2 transport),
  all-target Clippy with warnings denied passed, and the unchanged original
  Dekopon policy passed advisories, bans, licenses and sources. Receipts:
  `/tmp/writer-first-frame-embedded.log`,
  `/tmp/writer-embedded-upgrade-deny.log` and
  `/tmp/writer-dependency-upgrade-receipt.json`.
- **Integrated compile:** `cargo check -p claria-bedrock --all-targets --locked`
  passed, including billing and mock AWS consumers. Receipt:
  `/tmp/writer-claria-bedrock-check-upgrade.log`.
- **Eight broader service packages:** billing, core, docx, provisioner, records,
  report-store, storage and transcribe recorded **326 passed, 1 ignored, 0 failed**
  in `/tmp/writer-upgraded-services-tests.log`.
- **Remaining five non-desktop packages:** Claria, Bedrock, eval, DOCX CLI and
  mock AWS recorded **411 passed, 6 ignored, 0 failed** in
  `/tmp/writer-final-v3-verified-tests.log`. The initial run exposed two first-frame
  classification failures because the SDK now returns after headers. Both
  backends now explicitly cover send and first receive with one first-frame
  deadline, preserving the first event before switching to the idle bound.
  A raw first-event transient exception also verifies retry fidelity and sticky
  unknown usage. The initial failure receipt remains `/tmp/writer-final-v3-tests.log`.

These receipts cover their respective checkpoints, not live or native-desktop
acceptance. Cargo commands during shared work must use `--locked`;
an earlier non-locked run overwrote a concurrent lock patch, which was reapplied
and verified before the transport receipt above. No live credentials, paid model
calls, provisioning, or patient/private data were used. Fixtures do not establish
real-model writing quality, provider billing behavior or production performance.

The whole-Claria advisory scan still fails with **15 baseline diagnostics**:
seven vulnerability reports, seven unmaintained reports and one yanked package.
They are recorded in `/tmp/writer-claria-transport-advisory-summary.json` and
`/tmp/writer-claria-transport-advisories.log`:

- `crypto-bigint` 0.5.5:
  [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html), fixed
  in `>=0.7.6`.
- `quick-xml` 0.39.4:
  [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194.html) and
  [0195](https://rustsec.org/advisories/RUSTSEC-2026-0195.html), fixed in `>=0.41.0`.
- `zip` 8.6.0:
  [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html),
  [0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html) and
  [0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html), fixed in `>=9.0.0`.
- `webpki-root-certs` 1.0.9:
  [RUSTSEC-2026-0383](https://rustsec.org/advisories/RUSTSEC-2026-0383.html), fixed
  in `>=1.0.10`. It is absent from Bedrock and the host-desktop graph; its only
  resolved path is desktop → ureq → platform verifier under wasm32. It remains
  outside the SDK transport patch scope.
- Unmaintained `paste`, `proc-macro-error` and five `unic-*` packages; yanked
  `spin` 0.10.0.

No exceptions were added, and the embedded policy pass is not a claim that the
whole application is advisory-clean.

## Historical disposable probe

Before the real manifest updates, `/tmp/writer-dependency-probe` copied unchanged
source/tests from embedded checkpoint
`94acc810f6e96cb6f48ec10065fdab5f0c8a788d`, changed only config, BedrockRuntime,
Smithy types and Tokio pins, and regenerated its lock. Its **23 tests**, check,
all-target Clippy and original policy passed. Those 23 tests are historical probe
evidence, distinct from the active 35-test harness receipt above.

Probe lock SHA-256:
`5fec0aa258d0b40a808efc96eca67e8e89760e43185c6a989d469bb43fd6a20a`.
Original policy SHA-256:
`a4203329934519a9fa85d6312e4ab74a63ab2b78c0ab4f971500f20b8a329618`.
Copied-source checksums and exact commands remain in the probe's `SNAPSHOT.json`
and `CHECKS.json`; logs are `/tmp/writer-dependency-probe-{test,deny,check,clippy,tree}.log`.

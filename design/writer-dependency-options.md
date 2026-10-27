# Writer dependency options

Research and disposable compatibility probe, 2026-10-27. The actual Writer and
embedded manifests and locks retain the checkpoint pins. This document does not
waive the upstream policy or authorize a release.

## Tokio advisory

The current exact Tokio 1.52.3 pin is affected by
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html).
The advisory's patched ranges are `>=1.51.6, <1.52.0` or `>=1.53.3`; versions
before 0.3.2 are unaffected. The first forward fix is
[Tokio 1.53.3](https://github.com/tokio-rs/tokio/releases/tag/tokio-1.53.3), with
MSRV 1.71. The selected locked consumers' Tokio 1.x requirements accept it.
Tokio 1.54.1 is another option (MSRV 1.85), but adds a minor-version step. Avoid
yanked 1.54.0, which has the separate Interval race
[RUSTSEC-2026-0384](https://rustsec.org/advisories/RUSTSEC-2026-0384.html).

The minimal advisory-only fix is to move Claria's workspace pin and both embedded
normal/dev pins to 1.53.3 together, then regenerate their locks and rerun checks.
This alone does not resolve the AWS duplicate-type policy failure.

## AWS compatibility types

The current AWS SDK uses both `http` 0.2/1 and `http-body` 0.4/1 as compatibility
types. The active transport already uses Hyper 1, h2 0.4, rustls 0.23 and aws-lc;
the duplicates are not evidence of a second enabled legacy transport. No
feature-only fix exists for the exact current cohort.

AWS made the old HTTP types optional in its
[September 22, 2026 SDK migration](https://github.com/awslabs/aws-sdk-rust/blob/main/CHANGELOG.md#september-22nd-2026),
implemented by [Smithy PR 4827](https://github.com/smithy-lang/smithy-rs/pull/4827).
The first coherent embedded cohort is:

| Direct dependency | Current | Minimal cohort tested |
| --- | --- | --- |
| aws-config | 1.8.18 | 1.12.0 |
| aws-sdk-bedrockruntime | 1.135.0 | 1.146.0 |
| aws-smithy-types | 1.5.0 | 1.8.0 |
| tokio (normal and dev) | 1.52.3 | 1.53.3 |

That cohort's MSRV 1.94.1 fits the package's declared 1.97. Supporting versions
include AWS runtime 1.15.0, runtime API 1.17.0, HTTP client 1.4.2 and SigV4 1.6.0.
Upgrading only the embedded package would conflict with Claria's exact shared
pins or retain the legacy types through other AWS services. A full Claria move
also coordinates credentials 1.3.0, eventstream 0.61.4, Bedrock control 1.158.0,
S3 1.149.0, STS 1.117.0, Artifact 1.104.0, CloudTrail 1.123.0, Cost Explorer
1.130.0, IAM 1.126.0 and Transcribe 1.120.0, including billing/runtime and desktop
HTTP-client pins. Those broader consumers have not been upgraded or tested here.
The newer embedded cohort (config 1.13.0, Runtime 1.151.0, types 1.9.0) is an
alternative, but is unnecessary for the minimal policy fix and was not probed.

## Disposable probe evidence

The probe copied unchanged package source/tests from embedded checkpoint
`94acc810f6e96cb6f48ec10065fdab5f0c8a788d` to
`/tmp/writer-dependency-probe`, changed only the four direct versions above, and
regenerated its lock. It did not edit either real checkout or the policy.

- All 23 checkpoint tests passed with `cargo test --all-targets`.
- `cargo check --all-targets --locked` and all-target Clippy with warnings denied passed.
- `cargo deny --manifest-path /tmp/writer-dependency-probe/Cargo.toml --locked --config /workspace/dekopon-writer/deny.toml check` passed all four categories with the unchanged policy.
- The all-features, all-targets graph has HTTP 1.5.0, body 1.1.0, Hyper 1.12.0,
  h2 0.4.20, rustls 0.23.46 and aws-lc 1.18.2. HTTP 0.2 and body 0.4 are absent.

Resolved lock SHA-256:
`5fec0aa258d0b40a808efc96eca67e8e89760e43185c6a989d469bb43fd6a20a`.
Original policy SHA-256:
`a4203329934519a9fa85d6312e4ab74a63ab2b78c0ab4f971500f20b8a329618`.
Exact commands and copied-source checksums are in the disposable
`SNAPSHOT.json` and `CHECKS.json`; logs are `/tmp/writer-dependency-probe-{test,deny,check,clippy,tree}.log`.

This validates the first checkpoint's fixtures, not the active new kernel, the
whole Claria workspace, live Bedrock behavior or production performance. The
coherent AWS upgrade should be reviewed as a separate dependency change alongside
the ongoing extraction/error-sidecar release, rather than mixed into that release
or hidden behind a duplicate-policy exception.

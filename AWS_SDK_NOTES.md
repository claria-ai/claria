# AWS SDK Notes

## Binary size analysis (2026-02-28)

Release binary: **42.7 MiB** (16.8 MiB `.text` section).

Measured with `cargo bloat --release -p claria-desktop --crates`.

### Top crates by .text size

| Crate | Size | % of .text |
|---|---|---|
| `std` | 2.2 MiB | 13.1% |
| `aws_sdk_sts` | 1.2 MiB | 7.3% |
| `tauri` | 1.2 MiB | 6.9% |
| `aws_sdk_s3` | 1.0 MiB | 6.0% |
| `tokio` | 789 KiB | 4.6% |
| `aws_smithy_http_client` | 741 KiB | 4.3% |
| `rustls` | 684 KiB | 4.0% |
| `aws_lc_sys` | 673 KiB | 3.9% |
| `aws_config` | 482 KiB | 2.8% |
| `aws_sdk_bedrockruntime` | 466 KiB | 2.7% |
| `aws_sdk_iam` | 437 KiB | 2.5% |
| `aws_smithy_runtime` | 435 KiB | 2.5% |
| `h2` | 393 KiB | 2.3% |
| `claria_provisioner` | 383 KiB | 2.2% |
| `aws_sdk_bedrock` | 308 KiB | 1.8% |
| `claria_desktop` | 302 KiB | 1.7% |
| `aws_sdk_cloudtrail` | 286 KiB | 1.7% |
| `aws_smithy_types` | 217 KiB | 1.3% |
| `aws_smithy_runtime_api` | 210 KiB | 1.2% |

### Summary

- **AWS SDK service crates** (sts, s3, iam, bedrock, bedrockruntime, cloudtrail, artifact, sso/ssooidc): ~7.5 MiB (~45% of .text)
- **AWS SDK infrastructure** (smithy_http_client, smithy_runtime, smithy_types, smithy_runtime_api, aws_lc_sys, rustls, aws_config): ~3.2 MiB (~19% of .text)
- **AWS total**: ~10 MiB (~60% of .text)

Each AWS service SDK brings generated code for every API operation, request/response type, and error variant — even unused ones. STS is the largest single SDK despite only calling `GetCallerIdentity` and `AssumeRole`, because `aws-config` pulls in the full STS client for credential resolution.

### Mitigations applied (2026-02-28)

Added to workspace `Cargo.toml`:

```toml
[profile.release]
strip = true
lto = true
codegen-units = 1
opt-level = "z"
```

Result: **42.7 MiB → 11 MiB** (73% reduction). LTO enables cross-crate dead code elimination, which is particularly effective for the AWS SDKs since we only call a fraction of their generated API surface. `strip` removes debug symbols, `opt-level = "z"` optimizes for size. Trade-off: release builds take ~4 min instead of ~1 min.

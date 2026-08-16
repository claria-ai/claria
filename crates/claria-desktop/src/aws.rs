use std::{path::PathBuf, time::Duration};

use aws_config::{
    stalled_stream_protection::StalledStreamProtectionConfig, timeout::TimeoutConfig,
};

use crate::config::CredentialSource;

/// How long to wait for a TCP connection and TLS handshake.
///
/// Short on purpose: an unreachable endpoint should fail fast, and standard
/// retries (three attempts under the current behavior version) cover a
/// handshake lost to a flaky link. Slightly above the SDK's own 3.1s so a
/// clinician on hospital wifi does not lose a request to a slow handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for response headers after the request is sent.
///
/// Deliberately generous, because this is not the knob it looks like. The
/// SDK's read timeout wraps the future that resolves when the response head
/// arrives — it does not cover the body. Streaming calls get their headers
/// immediately and are unaffected either way, but the unary Converse calls
/// behind text extraction and translation send nothing at all until the
/// model has finished generating, and extraction alone may generate 16,384
/// tokens. A read timeout tuned to feel responsive would abort exactly the
/// long generations this release exists to protect.
///
/// So it is set to be unreachable by any legitimate call and still finite:
/// a connection that opened and then wedged fails in a quarter of an hour
/// with an error the clinician can see, instead of hanging for the life of
/// the process. The fast failure for an endpoint that is simply unreachable
/// is [`CONNECT_TIMEOUT`]'s job, not this one's.
pub const READ_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How long a transfer may deliver nothing before it is abandoned.
///
/// Must be set explicitly whenever a `StalledStreamProtectionConfig` is
/// supplied at all: the builder's own default is 20 seconds while the
/// runtime plugin's is 5, so an unset grace period silently quadruples it.
/// Ten seconds sits between the two — the throughput floor underneath this
/// is one byte per second, so only a genuinely dead socket trips it, and
/// the extra headroom keeps a stuttering upload alive.
pub const STALLED_STREAM_GRACE_PERIOD: Duration = Duration::from_secs(10);

/// The explicit timeout policy every AWS client in the app inherits.
///
/// Deliberately sets no operation or operation-attempt timeout. Both are
/// unset by default and both would cap total call duration, which is
/// exactly wrong here: a Bedrock stream writing a long clinical section
/// legitimately runs for many minutes, and an S3 transfer of imported audio
/// legitimately runs for minutes more. Connect and read timeouts bound the
/// phases that can hang without work happening; nothing bounds the phase
/// where work is happening.
///
/// One policy covers both Bedrock and S3. Their call profiles differ, but
/// not in any dimension this policy discriminates on — neither has a
/// defensible ceiling on total duration, and both want a fast connect. The
/// concrete cost of splitting them is that the shared HTTP client caches
/// connectors by `(connect_timeout, read_timeout)`, so per-service values
/// would fragment the connection pool the app keeps warm on purpose.
pub fn timeout_config() -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
}

/// Explicit stalled-stream protection, in both directions.
///
/// Note this does not reach `ConverseStream`: the generated streaming
/// Bedrock operations register no stalled-stream interceptor, so their
/// silence bounds live in `claria-bedrock` instead — one on the wait for
/// the first frame, one on the gaps between frames. This covers the unary
/// calls and every S3 transfer.
pub fn stalled_stream_protection() -> StalledStreamProtectionConfig {
    StalledStreamProtectionConfig::enabled()
        .grace_period(STALLED_STREAM_GRACE_PERIOD)
        .upload_enabled(true)
        .download_enabled(true)
        .build()
}

/// Build an `SdkConfig` from a region and credential source.
///
/// This is the only place in the desktop app that knows how to translate
/// a `CredentialSource` (our config-level type) into an AWS SDK config.
/// All AWS business logic lives in the provisioner — we just build the
/// config and hand it over.
pub async fn build_aws_config(region: &str, creds: &CredentialSource) -> aws_config::SdkConfig {
    // An explicit HTTP client shared by every service client built from this
    // config. Without one, each `Client::new` builds its own connector, so no
    // connection is ever reused across clients and every command pays
    // DNS/TCP/TLS setup on its first call.
    let http_client = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::AwsLc,
        ))
        .build_https();

    let mut builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .http_client(http_client)
        .timeout_config(timeout_config())
        .stalled_stream_protection(stalled_stream_protection());

    match creds {
        CredentialSource::Inline {
            access_key_id,
            secret_access_key,
            session_token,
        } => {
            builder = builder.credentials_provider(aws_sdk_sts::config::Credentials::new(
                access_key_id,
                secret_access_key,
                session_token.clone(),
                None,
                "claria-config",
            ));
        }
        CredentialSource::Profile { profile_name } => {
            builder = builder.profile_name(profile_name);
        }
        CredentialSource::DefaultChain => {}
    }

    builder.load().await
}

/// Parse AWS profile names from `~/.aws/credentials` and `~/.aws/config`.
pub fn list_aws_profiles() -> Vec<String> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };

    let aws_dir = home.join(".aws");
    let mut profiles = std::collections::BTreeSet::new();

    // Parse [profile_name] from credentials file
    parse_ini_sections(&aws_dir.join("credentials"), &mut profiles, false);

    // Parse [profile name] from config file
    parse_ini_sections(&aws_dir.join("config"), &mut profiles, true);

    // Remove "default" — it's implicit
    profiles.remove("default");

    profiles.into_iter().collect()
}

/// Parse INI-style section headers from an AWS config/credentials file.
/// If `strip_profile_prefix` is true, strips the `profile ` prefix from
/// section names (as used in `~/.aws/config`).
fn parse_ini_sections(
    path: &PathBuf,
    profiles: &mut std::collections::BTreeSet<String>,
    strip_profile_prefix: bool,
) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let mut name = trimmed[1..trimmed.len() - 1].trim().to_string();
            if strip_profile_prefix && let Some(stripped) = name.strip_prefix("profile ") {
                name = stripped.trim().to_string();
            }
            if !name.is_empty() {
                profiles.insert(name);
            }
        }
    }
}

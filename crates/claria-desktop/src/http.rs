//! Shared outbound HTTP(S) agent construction.
//!
//! Claria talks to non-AWS endpoints in exactly two places — the GitHub
//! release check and GGUF model downloads. Both build their agent here so
//! TLS verifies against the operating system's certificate store and reuses
//! the same rustls 0.23 / aws-lc crypto stack the AWS clients already link,
//! instead of pulling in a bundled CA list.

use std::{sync::Arc, time::Duration};

/// Build a `ureq` agent backed by rustls with the aws-lc crypto provider and
/// the platform certificate verifier.
///
/// `global_timeout` bounds the entire request including the body transfer, so
/// pass `None` for large downloads.
pub fn ureq_agent(global_timeout: Option<Duration>) -> ureq::Agent {
    let tls_config = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(global_timeout)
            .tls_config(tls_config)
            .build(),
    )
}

//! Release-version comparison for the update check.

/// True when `latest` is a strictly newer release than `current`.
///
/// Both sides are compared as semver. Lexicographic string comparison gets
/// this wrong the moment a component reaches two digits — "0.9.0" sorts
/// after "0.18.0" — which left everyone on an older minor believing they
/// were current.
///
/// An unparseable version on either side is reported as "no update" with a
/// warning: a malformed release tag must not nag every user into an upgrade
/// that may not exist.
pub fn update_available(current: &str, latest: &str) -> bool {
    match (parse(current, "current"), parse(latest, "latest")) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

fn parse(version: &str, which: &str) -> Option<semver::Version> {
    match semver::Version::parse(version) {
        Ok(parsed) => Some(parsed),
        Err(e) => {
            tracing::warn!(
                which,
                version,
                error = %e,
                "unparseable version in update check; treating as no update available"
            );
            None
        }
    }
}

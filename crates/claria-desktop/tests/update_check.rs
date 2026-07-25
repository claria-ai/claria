use claria_desktop::update::update_available;

#[test]
fn newer_minor_with_two_digits_beats_single_digit() {
    // The bug this fixes: lexicographically "0.9.0" > "0.18.0".
    assert!(update_available("0.9.0", "0.18.0"));
    assert!(!update_available("0.18.0", "0.9.0"));
}

#[test]
fn equal_versions_are_not_an_update() {
    assert!(!update_available("0.18.0", "0.18.0"));
    assert!(!update_available("1.0.0", "1.0.0"));
}

#[test]
fn patch_and_major_bumps_are_updates() {
    assert!(update_available("0.18.0", "0.18.1"));
    assert!(update_available("0.18.9", "1.0.0"));
}

#[test]
fn prerelease_sorts_below_its_release() {
    assert!(update_available("1.0.0-rc.1", "1.0.0"));
    assert!(!update_available("1.0.0", "1.0.0-rc.1"));
}

#[test]
fn malformed_versions_report_no_update() {
    assert!(!update_available("0.18.0", "not-a-version"));
    assert!(!update_available("not-a-version", "0.18.0"));
    assert!(!update_available("0.18.0", "v0.19.0"));
    assert!(!update_available("", ""));
}

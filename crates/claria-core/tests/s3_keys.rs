//! Tests for S3 key helpers: record search prefixes and sidecar hiding.

use std::collections::HashSet;

use claria_core::s3_keys;
use uuid::Uuid;

#[test]
fn search_prefix_extends_records_prefix() {
    let id = Uuid::nil();
    assert_eq!(
        s3_keys::client_records_search_prefix(id, "Dam"),
        format!("{}Dam", s3_keys::client_records_prefix(id)),
    );
}

#[test]
fn empty_search_prefix_equals_records_prefix() {
    let id = Uuid::nil();
    assert_eq!(
        s3_keys::client_records_search_prefix(id, ""),
        s3_keys::client_records_prefix(id),
    );
}

#[test]
fn sidecar_hidden_when_base_in_listing() {
    let keys: HashSet<&str> = ["records/x/Damien.pdf", "records/x/Damien.pdf.text"]
        .into_iter()
        .collect();
    assert!(s3_keys::is_hidden_sidecar(
        "records/x/Damien.pdf.text",
        &keys
    ));
    assert!(!s3_keys::is_hidden_sidecar("records/x/Damien.pdf", &keys));
}

#[test]
fn orphan_sidecar_visible() {
    let keys: HashSet<&str> = ["records/x/notes.txt.text"].into_iter().collect();
    assert!(!s3_keys::is_hidden_sidecar(
        "records/x/notes.txt.text",
        &keys
    ));
}

#[test]
fn sidecar_visible_when_prefix_excludes_base() {
    // Prefix "Damien.pdf.te" matches only the sidecar — the base file is not
    // in the filtered listing, so the sidecar is shown.
    let keys: HashSet<&str> = ["records/x/Damien.pdf.text"].into_iter().collect();
    assert!(!s3_keys::is_hidden_sidecar(
        "records/x/Damien.pdf.text",
        &keys
    ));
}

#[test]
fn short_prefix_keeps_sidecar_hidden() {
    // Any prefix no longer than the base filename that matches the sidecar
    // also matches the base, so both land in the listing and the sidecar
    // stays hidden.
    let base = "records/x/Damien.pdf";
    let sidecar = "records/x/Damien.pdf.text";
    for len in 0..=base.len() {
        let prefix = &sidecar[..len];
        assert!(base.starts_with(prefix));
        let keys: HashSet<&str> = [base, sidecar].into_iter().collect();
        assert!(s3_keys::is_hidden_sidecar(sidecar, &keys));
    }
}

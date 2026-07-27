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

// ── Audit trail keys ────────────────────────────────────────────────────────

#[test]
fn audit_event_key_nests_utc_date_and_leads_with_a_fixed_width_timestamp() {
    let ts: jiff::Timestamp = "2026-07-27T14:03:05.123456789Z".parse().expect("timestamp");
    let id: uuid::Uuid = "3f2b1c9e-5d4a-4b8e-9f01-2a3b4c5d6e7f"
        .parse()
        .expect("uuid");

    assert_eq!(
        s3_keys::audit_event(ts, id),
        "_audit/2026/07/27/20260727T140305.123456789Z-3f2b1c9e-5d4a-4b8e-9f01-2a3b4c5d6e7f.json"
    );
}

#[test]
fn audit_event_key_is_built_from_utc_not_local_time() {
    // 23:30 in Tokyo is the previous UTC day; the key must follow UTC so the
    // day prefix means the same thing everywhere.
    let ts: jiff::Timestamp = "2026-07-27T23:30:00Z".parse().expect("timestamp");
    let id = uuid::Uuid::nil();
    let key = s3_keys::audit_event(ts, id);
    assert!(key.starts_with("_audit/2026/07/27/"), "{key}");
}

#[test]
fn audit_event_keys_sort_lexicographically_by_time() {
    let id = uuid::Uuid::nil();
    let mut keys: Vec<String> = [
        "2026-07-27T14:03:05.000000002Z",
        "2026-01-02T03:04:05Z",
        "2026-07-27T14:03:05.000000001Z",
        "2026-07-27T09:00:00Z",
        "2025-12-31T23:59:59.999999999Z",
    ]
    .iter()
    .map(|s| s3_keys::audit_event(s.parse().expect("timestamp"), id))
    .collect();

    let chronological = vec![
        keys[4].clone(),
        keys[1].clone(),
        keys[3].clone(),
        keys[2].clone(),
        keys[0].clone(),
    ];
    keys.sort();
    assert_eq!(keys, chronological);
}

#[test]
fn audit_day_and_month_prefixes_bracket_the_event_key() {
    let ts: jiff::Timestamp = "2026-07-27T14:03:05Z".parse().expect("timestamp");
    let key = s3_keys::audit_event(ts, uuid::Uuid::nil());

    let date = jiff::civil::date(2026, 7, 27);
    assert_eq!(s3_keys::audit_day_prefix(date), "_audit/2026/07/27/");
    assert_eq!(s3_keys::audit_month_prefix(2026, 7), "_audit/2026/07/");

    assert!(key.starts_with(&s3_keys::audit_day_prefix(date)));
    assert!(key.starts_with(&s3_keys::audit_month_prefix(2026, 7)));
    assert!(key.starts_with(s3_keys::AUDIT_PREFIX));
}

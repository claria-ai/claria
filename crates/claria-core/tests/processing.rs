use claria_core::{
    processing::{ProcessingError, ProcessingFailureCode, usable_text},
    s3_keys::visible_record_files_with_errors,
};
use std::collections::HashSet;

#[test]
fn markers_are_canonical_versioned_and_distinct() {
    let first = ProcessingError::new(ProcessingFailureCode::ProviderUnavailable);
    let second = ProcessingError::new(first.code);
    assert_ne!(first.revision, second.revision);
    assert_eq!(
        ProcessingError::parse(&serde_json::to_vec(&first).unwrap()),
        Some(first.clone())
    );
    let mut forged = first;
    forged.reason = "private provider diagnostic".into();
    assert!(ProcessingError::parse(&serde_json::to_vec(&forged).unwrap()).is_none());
    assert!(ProcessingError::parse(b"ordinary user error notes").is_none());
}

#[test]
fn only_validated_errors_hide_and_supersede_text() {
    let keys = [
        "p/a.pdf",
        "p/a.pdf.text",
        "p/a.pdf.error",
        "p/user.error",
        "p/note",
        "p/note.text",
    ];
    let errors = HashSet::from(["p/a.pdf.error".to_string()]);
    let usable = HashSet::from(["p/a.pdf.text".to_string()]);
    let visible = visible_record_files_with_errors("p/", &keys, &errors, &usable);
    assert_eq!(
        visible
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<Vec<_>>(),
        ["a.pdf", "user.error", "note"]
    );
    assert_eq!(visible[0].source_index, 0);
    assert_eq!(visible[2].source_index, 4); // unusable text falls back to original
}

#[test]
fn extraction_requires_nonempty_printable_text() {
    assert!(usable_text(b" \n\t ").is_none());
    assert!(usable_text(b"%PDF-1.7").is_none());
    assert_eq!(usable_text(b"# Notes\nReadable"), Some("# Notes\nReadable"));
}

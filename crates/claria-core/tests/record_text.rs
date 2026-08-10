use claria_core::record_text::decode_record_text;

#[test]
fn accepts_structured_text_without_considering_its_filename() {
    let json = br#"{"instrument":"ADOS-2","score":12}"#;
    let markdown = "# Referral\n\nClient reports café visits.\n".as_bytes();
    let tabular = b"measure\tscore\r\nSRS-2\t71\r\n";

    assert_eq!(
        decode_record_text(json),
        Some(r#"{"instrument":"ADOS-2","score":12}"#)
    );
    assert_eq!(
        decode_record_text(markdown),
        Some("# Referral\n\nClient reports café visits.\n")
    );
    assert_eq!(
        decode_record_text(tabular),
        Some("measure\tscore\r\nSRS-2\t71\r\n")
    );
}

#[test]
fn rejects_invalid_or_control_heavy_bodies() {
    assert!(decode_record_text(b"valid prefix\0binary suffix").is_none());
    assert!(decode_record_text(b"escape\x1bsequence").is_none());
    assert!(decode_record_text(&[0xff, 0xfe, 0xfd]).is_none());
}

#[test]
fn rejects_printable_headers_for_binary_containers() {
    assert!(decode_record_text(b"%PDF-1.7\nprintable object syntax").is_none());
    assert!(decode_record_text(b"PK\x03\x04docx bytes").is_none());
    assert!(decode_record_text(b"0000ftypM4A bytes").is_none());
}

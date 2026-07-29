use std::{
    fs,
    io::{Cursor, Read},
    process::Command,
};

use claria_core::models::report::{ReportBlock, ReportContent, ReportDraft, ReportSection};
use claria_docx::render_report;
use quick_xml::{Reader, events::Event};
use zip::ZipArchive;

fn draft() -> ReportDraft {
    ReportDraft {
        revision: 7,
        content: ReportContent {
            title: "Evaluation & Plan <Final>".to_string(),
            sections: vec![
                ReportSection {
                    id: "11111111-1111-4111-8111-111111111111".parse().unwrap(),
                    heading: "History — 東京".to_string(),
                    blocks: vec![ReportBlock::Paragraph {
                        text: "First paragraph\nSecond café paragraph".to_string(),
                    }],
                },
                ReportSection {
                    id: "22222222-2222-4222-8222-222222222222".parse().unwrap(),
                    heading: "Recommendations".to_string(),
                    blocks: vec![ReportBlock::BulletList {
                        items: vec![
                            "Continue treatment".to_string(),
                            "Review in six weeks".to_string(),
                        ],
                    }],
                },
            ],
        },
        created_at: "2026-08-01T12:00:00Z".parse().unwrap(),
        updated_at: "2026-08-02T13:14:15Z".parse().unwrap(),
        last_applied_proposal_id: None,
    }
}

fn entry(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut file = archive.by_name(name).expect("entry");
    let mut value = String::new();
    file.read_to_string(&mut value).expect("utf-8 XML");
    value
}

#[test]
fn repeated_render_is_byte_identical_valid_docx() {
    let draft = draft();
    let first = render_report(&draft).expect("render");
    let second = render_report(&draft).expect("render again");
    assert_eq!(first, second);
    assert!(first.starts_with(b"PK"));

    let mut archive = ZipArchive::new(Cursor::new(&first)).expect("valid zip");
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
    ] {
        assert!(archive.by_name(required).is_ok(), "missing {required}");
    }
}

#[test]
fn every_xml_package_entry_is_strictly_well_formed() {
    let bytes = render_report(&draft()).expect("render");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut parsed = 0usize;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        if !name.ends_with(".xml") && !name.ends_with(".rels") {
            continue;
        }
        let mut xml = Vec::new();
        file.read_to_end(&mut xml).expect("read XML entry");
        let mut reader = Reader::from_reader(xml.as_slice());
        reader.config_mut().check_end_names = true;
        let mut buffer = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(Event::Eof) => break,
                Ok(_) => buffer.clear(),
                Err(error) => panic!("malformed OOXML entry {name}: {error}"),
            }
        }
        parsed += 1;
    }
    assert!(parsed >= 6, "expected core OOXML package entries");
}

#[test]
fn xml_illegal_control_characters_are_rejected_before_rendering() {
    let mut invalid = draft();
    invalid.content.sections[0].blocks[0] = ReportBlock::Paragraph {
        text: "invalid\u{000B}control".to_string(),
    };
    let error = render_report(&invalid).expect_err("illegal XML character");
    assert!(
        error
            .to_string()
            .contains("cannot be represented in Word XML")
    );
}

#[test]
fn document_contains_ordered_structured_content_and_page_setup() {
    let bytes = render_report(&draft()).expect("render");
    let document = entry(&bytes, "word/document.xml");

    let title = document
        .find("Evaluation &amp; Plan &lt;Final&gt;")
        .unwrap();
    let history = document.find("History — 東京").unwrap();
    let first = document.find("First paragraph").unwrap();
    let second = document.find("Second café paragraph").unwrap();
    let recommendations = document.find("Recommendations").unwrap();
    let bullet = document.find("Continue treatment").unwrap();
    assert!(title < history);
    assert!(history < first && first < second);
    assert!(second < recommendations && recommendations < bullet);

    assert!(document.contains(r#"<w:pgSz w:w="12240" w:h="15840" />"#));
    assert!(
        document.contains(r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440""#)
    );
    // Two source lines are separate paragraph elements, not one run with a
    // bullet character or raw newline.
    assert!(document.matches("<w:p ").count() >= 7);
}

#[test]
fn bullets_reference_real_ooxml_numbering() {
    let bytes = render_report(&draft()).expect("render");
    let document = entry(&bytes, "word/document.xml");
    let numbering = entry(&bytes, "word/numbering.xml");

    assert!(document.contains(r#"<w:numId w:val="42" />"#));
    assert!(document.contains("<w:numPr>"));
    assert!(numbering.contains(r#"<w:abstractNum w:abstractNumId="42""#));
    assert!(numbering.contains(r#"<w:numFmt w:val="bullet" />"#));
    assert!(numbering.contains(r#"<w:lvlText w:val="•" />"#));
    assert!(!document.contains("• Continue treatment"));
}

#[test]
fn fixed_styles_and_persisted_metadata_are_emitted() {
    let bytes = render_report(&draft()).expect("render");
    let styles = entry(&bytes, "word/styles.xml");
    let properties = entry(&bytes, "docProps/core.xml");

    assert!(styles.contains(r#"w:styleId="Normal""#));
    assert!(styles.contains(r#"w:styleId="Title""#));
    assert!(styles.contains(r#"w:styleId="Heading1""#));
    assert!(styles.contains("Times New Roman"));
    assert!(properties.contains("2026-08-01T12:00:00Z"));
    assert!(properties.contains("2026-08-02T13:14:15Z"));
}

#[test]
#[ignore = "requires LibreOffice to be installed"]
fn libreoffice_can_open_the_generated_document() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let document = directory.path().join("report.docx");
    fs::write(&document, render_report(&draft()).expect("render")).expect("write DOCX");
    let output = Command::new("libreoffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(directory.path())
        .arg(&document)
        .output()
        .expect("start LibreOffice");
    assert!(
        output.status.success(),
        "LibreOffice rejected DOCX: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("report.pdf").is_file());
}

#[test]
fn only_the_accepted_draft_is_rendered() {
    let mut accepted = draft();
    accepted.content.title = "Accepted title".to_string();
    let bytes = render_report(&accepted).expect("render");
    let document = entry(&bytes, "word/document.xml");
    assert!(document.contains("Accepted title"));
    assert!(!document.contains("Pending unaccepted title"));
}

use claria_bedrock::context::{build_context_block, ContextFile};

#[test]
fn empty_files_returns_empty_string() {
    assert_eq!(build_context_block(&[]), "");
}

#[test]
fn single_file_produces_valid_block() {
    let files = vec![ContextFile {
        filename: "notes.txt".to_string(),
        text: "Client presented with anxiety.".to_string(),
    }];

    let block = build_context_block(&files);
    assert!(block.starts_with("<record_context>"));
    assert!(block.ends_with("</record_context>"));
    assert!(block.contains("<file name=\"notes.txt\">"));
    assert!(block.contains("Client presented with anxiety."));
}

#[test]
fn multiple_files_all_included() {
    let files = vec![
        ContextFile {
            filename: "intake.txt".to_string(),
            text: "Intake notes here.\n".to_string(),
        },
        ContextFile {
            filename: "referral.pdf".to_string(),
            text: "Referral letter content.".to_string(),
        },
    ];

    let block = build_context_block(&files);
    assert!(block.contains("<file name=\"intake.txt\">"));
    assert!(block.contains("<file name=\"referral.pdf\">"));
    assert!(block.contains("Intake notes here."));
    assert!(block.contains("Referral letter content."));
}

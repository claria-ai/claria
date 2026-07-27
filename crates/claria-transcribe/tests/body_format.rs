//! Body format tests.
//!
//! Everything that starts from a body string is driven by the shared fixtures
//! in `fixtures/transcript-body/`, which the frontend's `transcript.test.ts`
//! reads from the same files — the two parsers mirror each other and nothing
//! else stops them drifting. See that directory's README.
//!
//! The tests below the fixture block start from a `TranscriptResult` instead,
//! which is the shape the AWS Transcribe parser produces and has no TypeScript
//! counterpart, so they stay hand-written.

use std::path::{Path, PathBuf};

use claria_transcribe::{
    Speaker, TranscriptResult, TranscriptSegment, format_transcript_body, parse_transcript_body,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExpectedBody {
    speaker_labels: Vec<String>,
    segments: Vec<ExpectedSegment>,
    rendered: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct ExpectedSegment {
    id: String,
    speaker_label: Option<String>,
    language_code: Option<String>,
    start_seconds: u32,
    end_seconds: u32,
    text: String,
    translation: Option<String>,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/transcript-body")
}

fn fixture_names() -> Vec<String> {
    let dir = fixture_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?;
            name.strip_suffix(".txt").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

fn load_fixture(name: &str) -> (String, ExpectedBody) {
    let dir = fixture_dir();
    let body = std::fs::read_to_string(dir.join(format!("{name}.txt")))
        .unwrap_or_else(|e| panic!("read {name}.txt: {e}"));
    let raw = std::fs::read_to_string(dir.join(format!("{name}.expected.json")))
        .unwrap_or_else(|e| panic!("read {name}.expected.json: {e}"));
    let expected =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {name}.expected.json: {e}"));
    (body, expected)
}

/// Flatten a parse result into the language-neutral shape the fixtures use:
/// the resolved speaker label rather than the interned `spk_N` id.
fn flatten(result: &TranscriptResult) -> Vec<ExpectedSegment> {
    result
        .segments
        .iter()
        .map(|seg| ExpectedSegment {
            id: seg.id.clone(),
            speaker_label: seg.speaker_id.as_deref().and_then(|id| {
                result
                    .speakers
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.label.clone())
            }),
            language_code: seg.language_code.clone(),
            start_seconds: seg.start_seconds,
            end_seconds: seg.end_seconds,
            text: seg.text.clone(),
            translation: seg.translation.clone(),
        })
        .collect()
}

#[test]
fn shared_fixtures_are_discovered() {
    // A moved or renamed directory would otherwise quietly turn the fixture
    // test below into zero assertions.
    assert!(
        !fixture_names().is_empty(),
        "no fixtures found in {}",
        fixture_dir().display()
    );
}

#[test]
fn shared_fixtures_parse_to_the_agreed_segments() {
    for name in fixture_names() {
        let (body, expected) = load_fixture(&name);
        let parsed = parse_transcript_body(&body);
        assert_eq!(flatten(&parsed), expected.segments, "fixture {name}");

        let labels: Vec<String> = parsed.speakers.iter().map(|s| s.label.clone()).collect();
        assert_eq!(labels, expected.speaker_labels, "fixture {name}");
    }
}

#[test]
fn shared_fixtures_render_back_to_the_agreed_body() {
    for name in fixture_names() {
        let (body, expected) = load_fixture(&name);
        let rendered = format_transcript_body(&parse_transcript_body(&body));
        assert_eq!(rendered, expected.rendered, "fixture {name}");

        // Rendering is a normalisation, so applying it twice must not keep
        // moving — an edit saves on every visit and drift would compound.
        let twice = format_transcript_body(&parse_transcript_body(&rendered));
        assert_eq!(twice, rendered, "fixture {name} is not round-trip stable");
    }
}

// ---------------------------------------------------------------------------
// Rendering from a TranscriptResult (no TypeScript counterpart)
// ---------------------------------------------------------------------------

fn diarized_sample() -> TranscriptResult {
    TranscriptResult {
        segments: vec![
            TranscriptSegment {
                id: "seg_0001".into(),
                speaker_id: Some("spk_0".into()),
                language_code: None,
                start_seconds: 0,
                end_seconds: 4,
                text: "How are you feeling today?".into(),
                translation: None,
            },
            TranscriptSegment {
                id: "seg_0002".into(),
                speaker_id: Some("spk_1".into()),
                language_code: None,
                start_seconds: 4,
                end_seconds: 9,
                text: "I've been having headaches for about a week.".into(),
                translation: None,
            },
        ],
        speakers: vec![
            Speaker {
                id: "spk_0".into(),
                label: "Clinician".into(),
            },
            Speaker {
                id: "spk_1".into(),
                label: "Patient".into(),
            },
        ],
    }
}

#[test]
fn renders_diarized_body_with_headers() {
    let body = format_transcript_body(&diarized_sample());
    assert!(body.contains("[Clinician 00:00\u{2013}00:04]"));
    assert!(body.contains("[Patient 00:04\u{2013}00:09]"));
    assert!(body.contains("How are you feeling today?"));
    assert!(body.contains("I've been having headaches"));
}

#[test]
fn renders_multilang_with_language_tag() {
    let result = TranscriptResult {
        segments: vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: Some("spk_0".into()),
            language_code: Some("es-US".into()),
            start_seconds: 9,
            end_seconds: 12,
            text: "¿En qué parte de la cabeza?".into(),
            translation: None,
        }],
        speakers: vec![Speaker {
            id: "spk_0".into(),
            label: "Clinician".into(),
        }],
    };
    let body = format_transcript_body(&result);
    assert!(body.contains("[Clinician 00:09\u{2013}00:12 es-US]"));
}

#[test]
fn round_trips_diarized_body() {
    let original = diarized_sample();
    let body = format_transcript_body(&original);
    let parsed = parse_transcript_body(&body);

    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.speakers.len(), 2);
    assert_eq!(parsed.segments[0].text, "How are you feeling today?");
    assert_eq!(
        parsed.segments[1].text,
        "I've been having headaches for about a week."
    );
    assert_eq!(parsed.segments[0].start_seconds, 0);
    assert_eq!(parsed.segments[0].end_seconds, 4);
    assert_eq!(parsed.speakers[0].label, "Clinician");
    assert_eq!(parsed.speakers[1].label, "Patient");
}

#[test]
fn renders_translation_as_blockquote() {
    let result = TranscriptResult {
        segments: vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: Some("spk_0".into()),
            language_code: Some("es-US".into()),
            start_seconds: 9,
            end_seconds: 12,
            text: "¿En qué parte de la cabeza?".into(),
            translation: Some("In what part of the head?".into()),
        }],
        speakers: vec![Speaker {
            id: "spk_0".into(),
            label: "Clinician".into(),
        }],
    };
    let body = format_transcript_body(&result);
    assert!(body.contains("¿En qué parte de la cabeza?"));
    assert!(body.contains("> In what part of the head?"));
}

#[test]
fn round_trips_translation_through_body() {
    let original = TranscriptResult {
        segments: vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: Some("spk_0".into()),
            language_code: Some("es-US".into()),
            start_seconds: 0,
            end_seconds: 5,
            text: "Hola, ¿cómo está?".into(),
            translation: Some("Hello, how are you?".into()),
        }],
        speakers: vec![Speaker {
            id: "spk_0".into(),
            label: "Clinician".into(),
        }],
    };
    let body = format_transcript_body(&original);
    let parsed = parse_transcript_body(&body);

    assert_eq!(parsed.segments.len(), 1);
    assert_eq!(parsed.segments[0].text, "Hola, ¿cómo está?");
    assert_eq!(
        parsed.segments[0].translation.as_deref(),
        Some("Hello, how are you?")
    );
}

#[test]
fn no_diarization_no_language_renders_without_headers() {
    let result = TranscriptResult {
        segments: vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: None,
            language_code: None,
            start_seconds: 0,
            end_seconds: 5,
            text: "Patient was seen today.".into(),
            translation: None,
        }],
        speakers: vec![],
    };
    let body = format_transcript_body(&result);
    assert_eq!(body, "Patient was seen today.");
}

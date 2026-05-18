use claria_transcribe::{
    Speaker, TranscriptResult, TranscriptSegment, format_transcript_body, parse_transcript_body,
};

fn diarized_sample() -> TranscriptResult {
    TranscriptResult {
        segments: vec![
            TranscriptSegment {
                id: "seg_0001".into(),
                speaker_id: Some("spk_0".into()),
                language_code: None,
                start_ms: 0,
                end_ms: 4_200,
                text: "How are you feeling today?".into(),
            },
            TranscriptSegment {
                id: "seg_0002".into(),
                speaker_id: Some("spk_1".into()),
                language_code: None,
                start_ms: 4_200,
                end_ms: 9_000,
                text: "I've been having headaches for about a week.".into(),
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
            start_ms: 9_000,
            end_ms: 12_500,
            text: "¿En qué parte de la cabeza?".into(),
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
    assert_eq!(parsed.segments[0].start_ms, 0);
    assert_eq!(parsed.segments[0].end_ms, 4_000); // mm:ss truncates ms
    assert_eq!(parsed.speakers[0].label, "Clinician");
    assert_eq!(parsed.speakers[1].label, "Patient");
}

#[test]
fn legacy_header_less_body_parses_as_single_segment() {
    let body = "Patient was seen in the office today for follow-up.\nNo new complaints.";
    let parsed = parse_transcript_body(body);

    assert_eq!(parsed.segments.len(), 1);
    assert_eq!(parsed.segments[0].speaker_id, None);
    assert_eq!(parsed.segments[0].language_code, None);
    assert!(parsed.segments[0].text.contains("Patient was seen"));
    assert!(parsed.segments[0].text.contains("No new complaints."));
}

#[test]
fn hand_edited_speaker_label_round_trips() {
    let body = "\
[Dr. Smith 00:00\u{2013}00:04]
How are you feeling today?

[Patient 00:04\u{2013}00:09]
Better, thank you.
";
    let parsed = parse_transcript_body(body);

    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.speakers.len(), 2);
    assert_eq!(parsed.speakers[0].label, "Dr. Smith");
    assert_eq!(parsed.speakers[0].id, "spk_0");
    assert_eq!(parsed.speakers[1].label, "Patient");
}

#[test]
fn ascii_hyphen_in_header_is_tolerated() {
    let body = "[Clinician 00:00-00:04]\nHello.";
    let parsed = parse_transcript_body(body);
    assert_eq!(parsed.segments.len(), 1);
    assert_eq!(parsed.segments[0].start_ms, 0);
    assert_eq!(parsed.segments[0].end_ms, 4_000);
    assert_eq!(parsed.segments[0].text, "Hello.");
}

#[test]
fn shared_speaker_label_collapses_to_one_speaker() {
    let body = "\
[Clinician 00:00\u{2013}00:04]
First.

[Clinician 00:10\u{2013}00:14]
Second.
";
    let parsed = parse_transcript_body(body);
    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.speakers.len(), 1);
    assert_eq!(parsed.segments[0].speaker_id, parsed.segments[1].speaker_id);
}

#[test]
fn empty_body_produces_no_segments() {
    let parsed = parse_transcript_body("");
    assert!(parsed.segments.is_empty());
    assert!(parsed.speakers.is_empty());
}

#[test]
fn no_diarization_no_language_renders_without_headers() {
    let result = TranscriptResult {
        segments: vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: None,
            language_code: None,
            start_ms: 0,
            end_ms: 5_000,
            text: "Patient was seen today.".into(),
        }],
        speakers: vec![],
    };
    let body = format_transcript_body(&result);
    assert_eq!(body, "Patient was seen today.");
}

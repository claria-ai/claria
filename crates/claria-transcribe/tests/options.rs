use claria_transcribe::{LanguageMode, SpeakerHandling, TranscribeOptions, TranscriptionEngine};

#[test]
fn default_options_are_english_standard_two_speakers() {
    let opts = TranscribeOptions::default();
    assert_eq!(opts.language, LanguageMode::English);
    assert_eq!(opts.engine, TranscriptionEngine::Standard);
    assert!(matches!(opts.speakers, SpeakerHandling::Diarize { max: 2 }));
}

#[test]
fn options_serialize_snake_case() {
    let opts = TranscribeOptions {
        language: LanguageMode::Mixed,
        speakers: SpeakerHandling::Diarize { max: 3 },
        engine: TranscriptionEngine::Medical,
    };
    let json = serde_json::to_string(&opts).unwrap();
    assert!(json.contains("\"language\":\"mixed\""));
    assert!(json.contains("\"engine\":\"medical\""));
    assert!(json.contains("\"kind\":\"diarize\""));
    assert!(json.contains("\"max\":3"));
}

#[test]
fn options_round_trip_through_json() {
    let opts = TranscribeOptions {
        language: LanguageMode::Spanish,
        speakers: SpeakerHandling::Channels,
        engine: TranscriptionEngine::Standard,
    };
    let json = serde_json::to_string(&opts).unwrap();
    let back: TranscribeOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(opts, back);
}

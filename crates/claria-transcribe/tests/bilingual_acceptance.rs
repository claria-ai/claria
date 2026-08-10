//! Acceptance test for bilingual (Mixed-language) transcription.
//!
//! Drives `transcribe_audio_with_options` end-to-end against `claria-mock-aws`
//! so we can assert on *both* halves of the bilingual bug surface:
//!
//!   1. The request the SDK sent to AWS Transcribe was shaped correctly for
//!      code-switching — `IdentifyMultipleLanguages: true`,
//!      `LanguageOptions: [en-US, es-US]`, speaker diarization, no pinned
//!      `LanguageCode`.
//!   2. The recorded multi-language transcript JSON parses into a
//!      `TranscriptResult` whose segments carry both `en-US` and `es-US`
//!      language codes, with each speaker mono-lingual.
//!
//! The audio fixture (`tests/assets/don_quijote_intro_two_speakers.m4a`) is a
//! recording of the opening of Don Quijote read once in Spanish by one speaker
//! and once in English by another. The cassette
//! (`tests/assets/don_quijote_intro_two_speakers.transcribe.json`) is the
//! *real* AWS Transcribe Mixed-mode output for that audio — captured by
//! running `cargo run --example transcribe_m4a -- <bucket>` against the same
//! file and committing the dumped JSON. It contains AWS's real `language_codes`
//! array (229.44 s en-US, 157.91 s es-US), 1107 items individually tagged by
//! language, and 25 speaker-diarized segments. Refresh by re-running the same
//! example and overwriting the file.

use std::{collections::HashSet, path::Path};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_transcribe::types::MediaFormat;

use claria_mock_aws::testing::MockServer;
use claria_transcribe::{
    LanguageMode, SpeakerHandling, TranscribeOptions, TranscriptionEngine,
    transcribe_audio_with_options,
};

const AUDIO_FILENAME: &str = "don_quijote_intro_two_speakers.m4a";
const AUDIO_KEY: &str = "records/don-quijote/don_quijote_intro_two_speakers.m4a";
const BUCKET: &str = "claria-test-bucket";
const CASSETTE_RELATIVE: &str = "tests/assets/don_quijote_intro_two_speakers.transcribe.json";

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(creds))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn upload_audio(sdk: &aws_config::SdkConfig) {
    let s3 = claria_storage::client::from_config(sdk);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    let audio_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("assets")
        .join(AUDIO_FILENAME);
    let bytes = std::fs::read(&audio_path).expect("read audio fixture");
    s3.put_object()
        .bucket(BUCKET)
        .key(AUDIO_KEY)
        .body(ByteStream::from(bytes))
        .send()
        .await
        .expect("put audio");
}

fn load_cassette() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CASSETTE_RELATIVE);
    let raw = std::fs::read_to_string(&path).expect("read cassette");
    serde_json::from_str(&raw).expect("parse cassette")
}

#[tokio::test]
async fn mixed_mode_request_uses_language_identification_and_returns_both_languages() {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);

    upload_audio(&sdk).await;

    {
        let mut st = mock.state.write().await;
        st.transcribe_response_cassette.push(load_cassette());
    }

    let options = TranscribeOptions {
        language: LanguageMode::Mixed,
        speakers: SpeakerHandling::Diarize { max: 2 },
        engine: TranscriptionEngine::Standard,
    };

    let result = transcribe_audio_with_options(&sdk, BUCKET, AUDIO_KEY, MediaFormat::Mp4, &options)
        .await
        .expect("transcription succeeded");

    // ── Assert on the request the SDK actually sent ───────────────────────────
    let recorded = {
        let st = mock.state.read().await;
        st.transcribe_requests.clone()
    };
    assert_eq!(
        recorded.len(),
        1,
        "expected exactly one StartTranscriptionJob, got {}",
        recorded.len()
    );
    let req = &recorded[0];
    assert_eq!(req.operation, "StartTranscriptionJob");

    assert_eq!(
        req.body.get("LanguageCode"),
        None,
        "Mixed mode must not pin a single LanguageCode; full request body: {}",
        req.body
    );
    assert_eq!(
        req.body
            .get("IdentifyMultipleLanguages")
            .and_then(|v| v.as_bool()),
        Some(true),
        "Mixed mode must set IdentifyMultipleLanguages=true; full request body: {}",
        req.body
    );
    let lang_options: Vec<&str> = req
        .body
        .get("LanguageOptions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        lang_options.contains(&"en-US") && lang_options.contains(&"es-US"),
        "Mixed mode must send both en-US and es-US in LanguageOptions; got {lang_options:?}"
    );

    assert_eq!(
        req.body
            .get("Settings")
            .and_then(|s| s.get("ShowSpeakerLabels"))
            .and_then(|v| v.as_bool()),
        Some(true),
        "diarization should be enabled; full request body: {}",
        req.body
    );
    assert_eq!(
        req.body
            .get("Settings")
            .and_then(|s| s.get("MaxSpeakerLabels"))
            .and_then(|v| v.as_i64()),
        Some(2),
        "max speakers should be 2; full request body: {}",
        req.body
    );

    assert_eq!(
        req.body.get("MediaFormat").and_then(|v| v.as_str()),
        Some("mp4"),
        "MediaFormat should reflect the .m4a input; full request body: {}",
        req.body
    );

    // ── Assert on the parsed result ───────────────────────────────────────────
    assert_eq!(result.speakers.len(), 2, "expected two speakers");

    // Segments must be chronological. AWS returns speaker_labels.segments
    // grouped by speaker, not by time, so the parser is responsible for the
    // sort. If this regresses, the UI renders segments out of order.
    let starts: Vec<u32> = result.segments.iter().map(|s| s.start_seconds).collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(
        starts, sorted,
        "segments must be returned in chronological order by start_seconds"
    );
    let ids: Vec<&str> = result.segments.iter().map(|s| s.id.as_str()).collect();
    let expected_ids: Vec<String> = (1..=result.segments.len())
        .map(|i| format!("seg_{i:04}"))
        .collect();
    assert_eq!(
        ids,
        expected_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "post-sort IDs should be monotonic starting at seg_0001"
    );

    let langs: HashSet<&str> = result
        .segments
        .iter()
        .filter_map(|s| s.language_code.as_deref())
        .collect();
    assert!(
        langs.contains("en-US") && langs.contains("es-US"),
        "expected both en-US and es-US among segment language codes; got {langs:?}"
    );

    // In this recording each reader speaks exactly one language, so every
    // speaker's segments should be mono-lingual. If a future parser change
    // started mixing items across speaker turns this would catch it.
    use std::collections::HashMap;
    let mut langs_per_speaker: HashMap<&str, HashSet<&str>> = HashMap::new();
    for seg in &result.segments {
        if let (Some(spk), Some(lang)) = (seg.speaker_id.as_deref(), seg.language_code.as_deref()) {
            langs_per_speaker.entry(spk).or_default().insert(lang);
        }
    }
    for (spk, langs) in &langs_per_speaker {
        assert_eq!(
            langs.len(),
            1,
            "speaker {spk} should be mono-lingual in this recording, got {langs:?}"
        );
    }
    let distinct_speaker_langs: HashSet<&&str> = langs_per_speaker.values().flatten().collect();
    assert_eq!(
        distinct_speaker_langs.len(),
        2,
        "the two speakers should split en-US / es-US between them, got {langs_per_speaker:?}"
    );
}

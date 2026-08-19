use std::path::Path;

use claria_transcribe::{LocalTranscribeOptions, LocalTranscriber};

#[test]
fn local_runtime_rejects_empty_pcm_before_loading_a_model() {
    let error = LocalTranscriber::default()
        .transcribe_pcm(
            Path::new("model-does-not-exist.gguf"),
            &[],
            &LocalTranscribeOptions::default(),
        )
        .unwrap_err();

    assert_eq!(error.to_string(), "audio contains no samples");
}

#[test]
fn local_runtime_rejects_non_finite_pcm_before_loading_a_model() {
    let error = LocalTranscriber::default()
        .transcribe_pcm(
            Path::new("model-does-not-exist.gguf"),
            &[f32::NAN],
            &LocalTranscribeOptions::default(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("non-finite sample"));
}

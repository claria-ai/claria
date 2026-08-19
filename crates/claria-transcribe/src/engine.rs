use std::path::{Path, PathBuf};

use transcribe_cpp::{
    Model, ModelOptions, RunExtension, RunOptions, SessionOptions, TimestampKind, WhisperRunOptions,
};

use crate::{
    LanguageMode,
    error::TranscribeError,
    types::{InferenceTimings, LocalTranscribeOptions, LocalTranscription},
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelKey {
    path: PathBuf,
    backend: crate::types::InferenceBackend,
    gpu_device: i32,
}

#[derive(Debug)]
struct CachedModel {
    key: ModelKey,
    model: Model,
}

/// A reusable transcribe.cpp model cache.
///
/// The desktop keeps one instance behind a mutex. Switching model or backend
/// evicts the previous native model so large GGUFs do not accumulate in RAM.
#[derive(Debug, Default)]
pub struct LocalTranscriber {
    cached: Option<CachedModel>,
}

impl LocalTranscriber {
    pub fn clear(&mut self) {
        self.cached = None;
    }

    pub fn transcribe_pcm(
        &mut self,
        model_path: &Path,
        pcm_16khz: &[f32],
        options: &LocalTranscribeOptions,
    ) -> Result<LocalTranscription, TranscribeError> {
        validate_options(options)?;
        if pcm_16khz.is_empty() {
            return Err(TranscribeError::EmptyAudio);
        }
        if pcm_16khz.iter().any(|sample| !sample.is_finite()) {
            return Err(TranscribeError::InvalidOption(
                "PCM input contains a non-finite sample".to_string(),
            ));
        }

        let model = self.model(model_path, options)?;
        let architecture = model.arch();
        let variant = model.variant();
        let backend = model.backend();
        let timestamps = if model.capabilities().max_timestamp_kind == TimestampKind::None {
            TimestampKind::None
        } else {
            TimestampKind::Segment
        };
        let native = run_native(model, pcm_16khz, options, &architecture, timestamps)?;
        let timings = inference_timings(&native);
        let text = native.text.trim().to_string();
        let detected_language = native.language;

        tracing::info!(
            model_architecture = architecture,
            model_variant = variant,
            backend,
            segments = native.segments.len(),
            detected_language = ?detected_language,
            "local transcription complete"
        );

        Ok(LocalTranscription {
            text,
            detected_language,
            model_architecture: architecture,
            model_variant: variant,
            backend,
            timings,
        })
    }

    fn model(
        &mut self,
        model_path: &Path,
        options: &LocalTranscribeOptions,
    ) -> Result<&Model, TranscribeError> {
        let key = ModelKey {
            path: model_path.to_path_buf(),
            backend: options.backend,
            gpu_device: options.gpu_device,
        };
        let must_load = self.cached.as_ref().is_none_or(|cached| cached.key != key);

        if must_load {
            tracing::info!(
                path = %model_path.display(),
                backend = ?options.backend,
                gpu_device = options.gpu_device,
                "loading transcribe.cpp model"
            );
            let model = Model::load_with(
                model_path,
                &ModelOptions {
                    backend: options.backend.to_native(),
                    gpu_device: options.gpu_device,
                },
            )
            .map_err(|error| TranscribeError::ModelLoad(error.to_string()))?;
            tracing::info!(
                architecture = model.arch(),
                variant = model.variant(),
                backend = model.backend(),
                "transcribe.cpp model loaded"
            );
            self.cached = Some(CachedModel { key, model });
        }

        self.cached
            .as_ref()
            .map(|cached| &cached.model)
            .ok_or_else(|| TranscribeError::ModelLoad("model cache is empty".to_string()))
    }
}

fn run_native(
    model: &Model,
    pcm_16khz: &[f32],
    options: &LocalTranscribeOptions,
    architecture: &str,
    timestamps: TimestampKind,
) -> Result<transcribe_cpp::Transcript, TranscribeError> {
    let mut session = model
        .session_with(&SessionOptions {
            n_threads: options.n_threads,
            kv_type: options.kv_precision.to_native(),
            n_ctx: 0,
        })
        .map_err(|error| TranscribeError::Inference(error.to_string()))?;

    let family = if architecture == "whisper" {
        let whisper = &options.whisper;
        Some(RunExtension::Whisper(WhisperRunOptions {
            initial_prompt: non_empty(&whisper.initial_prompt),
            condition_on_prev_tokens: Some(whisper.condition_on_previous_text),
            temperature: Some(whisper.temperature),
            temperature_inc: Some(whisper.temperature_increment),
            compression_ratio_thold: Some(whisper.compression_ratio_threshold),
            logprob_thold: Some(whisper.log_probability_threshold),
            no_speech_thold: Some(whisper.no_speech_threshold),
            max_prev_context_tokens: Some(whisper.max_previous_context_tokens),
            seed: Some(whisper.seed),
            max_initial_timestamp: None,
        }))
    } else {
        None
    };

    let run_options = RunOptions {
        timestamps,
        language: language_hint(options.language, architecture),
        // Curated Whisper models do not advertise speculative decoding.
        spec_k_drafts: -1,
        family,
        ..RunOptions::default()
    };

    session
        .run(pcm_16khz, &run_options)
        .map_err(|error| TranscribeError::Inference(error.to_string()))
}

fn inference_timings(native: &transcribe_cpp::Transcript) -> InferenceTimings {
    InferenceTimings {
        load_ms: native.timings.load_ms,
        mel_ms: native.timings.mel_ms,
        encode_ms: native.timings.encode_ms,
        decode_ms: native.timings.decode_ms,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn language_hint(language: LanguageMode, architecture: &str) -> Option<String> {
    if architecture != "whisper" {
        return None;
    }
    match language {
        LanguageMode::English => Some("en".to_string()),
        LanguageMode::Spanish => Some("es".to_string()),
        LanguageMode::Mixed => None,
    }
}

fn validate_options(options: &LocalTranscribeOptions) -> Result<(), TranscribeError> {
    if options.gpu_device < 0 {
        return Err(TranscribeError::InvalidOption(
            "GPU device index cannot be negative".to_string(),
        ));
    }
    if !(0..=256).contains(&options.n_threads) {
        return Err(TranscribeError::InvalidOption(
            "CPU threads must be between 0 and 256".to_string(),
        ));
    }
    let whisper = &options.whisper;
    if whisper.initial_prompt.contains('\0') {
        return Err(TranscribeError::InvalidOption(
            "initial prompt cannot contain a NUL character".to_string(),
        ));
    }
    if !(0..=448).contains(&whisper.max_previous_context_tokens) {
        return Err(TranscribeError::InvalidOption(
            "previous context token limit must be between 0 and 448".to_string(),
        ));
    }
    validate_f32("temperature", whisper.temperature, 0.0, 1.0)?;
    validate_f32(
        "temperature increment",
        whisper.temperature_increment,
        0.0,
        1.0,
    )?;
    validate_f32(
        "compression ratio threshold",
        whisper.compression_ratio_threshold,
        0.1,
        100.0,
    )?;
    validate_f32(
        "log probability threshold",
        whisper.log_probability_threshold,
        -100.0,
        0.0,
    )?;
    validate_f32("no-speech threshold", whisper.no_speech_threshold, 0.0, 1.0)?;
    Ok(())
}

fn validate_f32(name: &str, value: f32, min: f32, max: f32) -> Result<(), TranscribeError> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(TranscribeError::InvalidOption(format!(
            "{name} must be between {min} and {max}"
        )));
    }
    Ok(())
}

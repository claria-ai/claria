//! claria-whisper
//!
//! Local audio transcription using candle (pure Rust).

pub mod error;

use std::path::Path;

use byteorder::{ByteOrder, LittleEndian};
use candle_core::{Device, IndexOp, Tensor};
use candle_nn::{ops::softmax, VarBuilder};
use candle_transformers::models::whisper::{self as m, audio, Config};
use tokenizers::Tokenizer;
use tracing::info;

use crate::error::WhisperError;

static MEL_FILTERS: &[u8] = include_bytes!("melfilters.bytes");

/// A loaded Whisper model ready for transcription.
///
/// Load once via [`WhisperModel::load`], then call [`WhisperModel::transcribe`]
/// repeatedly without reloading weights from disk.
pub struct WhisperModel {
    model: m::model::Whisper,
    config: Config,
    tokenizer: Tokenizer,
    mel_filters: Vec<f32>,
    sot_token: u32,
    eot_token: u32,
    transcribe_token: u32,
    no_timestamps_token: u32,
    no_speech_token: Option<u32>,
    suppress_tokens: Tensor,
}

impl WhisperModel {
    /// Load a Whisper model from `model_dir`.
    ///
    /// `model_dir` must contain `model.safetensors`, `config.json`, and
    /// `tokenizer.json` (the openai/whisper-base.en layout from Hugging Face).
    pub fn load(model_dir: &Path) -> Result<Self, WhisperError> {
        let device = Device::Cpu;

        // Load config
        let config_path = model_dir.join("config.json");
        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| WhisperError::ModelLoad(format!("reading config.json: {e}")))?;
        let config: Config = serde_json::from_str(&config_str)
            .map_err(|e| WhisperError::ModelLoad(format!("parsing config.json: {e}")))?;

        // Load tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| WhisperError::Tokenizer(format!("loading tokenizer.json: {e}")))?;

        // Load mel filters (embedded at compile time)
        let mut mel_filters = vec![0f32; MEL_FILTERS.len() / 4];
        LittleEndian::read_f32_into(MEL_FILTERS, &mut mel_filters);

        // Load model weights
        let weights_path = model_dir.join("model.safetensors");
        info!(path = %weights_path.display(), "loading whisper model");
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], m::DTYPE, &device)
                .map_err(|e| WhisperError::ModelLoad(e.to_string()))?
        };
        let model = m::model::Whisper::load(&vb, config.clone())
            .map_err(|e| WhisperError::ModelLoad(e.to_string()))?;

        // Resolve special tokens
        let sot_token = token_id(&tokenizer, m::SOT_TOKEN)?;
        let eot_token = token_id(&tokenizer, m::EOT_TOKEN)?;
        let transcribe_token = token_id(&tokenizer, m::TRANSCRIBE_TOKEN)?;
        let no_timestamps_token = token_id(&tokenizer, m::NO_TIMESTAMPS_TOKEN)?;
        let no_speech_token = m::NO_SPEECH_TOKENS
            .iter()
            .find_map(|t| token_id(&tokenizer, t).ok());

        // Build suppress-tokens mask
        let suppress_tokens: Vec<f32> = (0..config.vocab_size as u32)
            .map(|i| {
                if config.suppress_tokens.contains(&i) {
                    f32::NEG_INFINITY
                } else {
                    0f32
                }
            })
            .collect();
        let suppress_tokens = Tensor::new(suppress_tokens.as_slice(), &device)
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        info!("whisper model loaded");

        Ok(Self {
            model,
            config,
            tokenizer,
            mel_filters,
            sot_token,
            eot_token,
            transcribe_token,
            no_timestamps_token,
            no_speech_token,
            suppress_tokens,
        })
    }

    /// Transcribe 16 kHz mono f32 PCM audio.
    pub fn transcribe(&mut self, pcm_16khz: &[f32]) -> Result<String, WhisperError> {
        let device = Device::Cpu;
        let duration_secs = pcm_16khz.len() as f64 / 16000.0;
        info!(
            samples = pcm_16khz.len(),
            duration_secs = format!("{duration_secs:.1}"),
            "transcribing audio"
        );

        // Convert PCM to mel spectrogram
        let mel = audio::pcm_to_mel(&self.config, pcm_16khz, &self.mel_filters);
        let mel_len = mel.len();
        let mel = Tensor::from_vec(
            mel,
            (1, self.config.num_mel_bins, mel_len / self.config.num_mel_bins),
            &device,
        )
        .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        // Decode all segments
        let (_, _, content_frames) = mel
            .dims3()
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        let total_segments = content_frames.div_ceil(m::N_FRAMES);
        info!(content_frames, total_segments, "decoding segments");

        let mut seek = 0;
        let mut segment_idx = 0;
        let mut full_text = String::new();

        while seek < content_frames {
            let segment_size = usize::min(content_frames - seek, m::N_FRAMES);
            segment_idx += 1;
            let mel_segment = mel
                .narrow(2, seek, segment_size)
                .map_err(|e| WhisperError::Transcription(e.to_string()))?;
            seek += segment_size;

            // Encode audio
            let audio_features = self
                .model
                .encoder
                .forward(&mel_segment, true)
                .map_err(|e| WhisperError::Transcription(e.to_string()))?;

            // Greedy decode (English, no timestamps)
            let sample_len = self.config.max_target_positions / 2;
            let mut tokens = vec![
                self.sot_token,
                self.transcribe_token,
                self.no_timestamps_token,
            ];

            for i in 0..sample_len {
                let tokens_t = Tensor::new(tokens.as_slice(), &device)
                    .and_then(|t| t.unsqueeze(0))
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let ys = self
                    .model
                    .decoder
                    .forward(&tokens_t, &audio_features, i == 0)
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let (_, seq_len, _) = ys
                    .dims3()
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let logits = self
                    .model
                    .decoder
                    .final_linear(
                        &ys.i((..1, seq_len - 1..))
                            .map_err(|e| WhisperError::Transcription(e.to_string()))?,
                    )
                    .and_then(|t| t.i(0))
                    .and_then(|t| t.i(0))
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                // Apply suppress-tokens mask and pick argmax
                let logits = logits
                    .broadcast_add(&self.suppress_tokens)
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let logits_v: Vec<f32> = logits
                    .to_vec1()
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let next_token = logits_v
                    .iter()
                    .enumerate()
                    .max_by(|(_, u), (_, v)| u.total_cmp(v))
                    .map(|(i, _)| i as u32)
                    .ok_or_else(|| WhisperError::Transcription("empty logits".into()))?;

                if next_token == self.eot_token
                    || tokens.len() > self.config.max_target_positions
                {
                    break;
                }
                tokens.push(next_token);
            }

            // Check for no-speech
            if let Some(nst) = self.no_speech_token {
                let first_tokens = Tensor::new(
                    &[
                        self.sot_token,
                        self.transcribe_token,
                        self.no_timestamps_token,
                    ],
                    &device,
                )
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let first_ys = self
                    .model
                    .decoder
                    .forward(&first_tokens, &audio_features, true)
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let first_logits = self
                    .model
                    .decoder
                    .final_linear(
                        &first_ys
                            .i(..1)
                            .map_err(|e| WhisperError::Transcription(e.to_string()))?,
                    )
                    .and_then(|t| t.i(0))
                    .and_then(|t| t.i(0))
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?;

                let no_speech_prob = softmax(&first_logits, 0)
                    .and_then(|s| s.i(nst as usize))
                    .and_then(|s| s.to_scalar::<f32>())
                    .unwrap_or(0.0) as f64;

                if no_speech_prob > m::NO_SPEECH_THRESHOLD {
                    continue;
                }
            }

            let text = self.tokenizer.decode(&tokens, true).map_err(|e| {
                WhisperError::Tokenizer(format!("decoding tokens: {e}"))
            })?;

            info!(
                segment = segment_idx,
                total_segments,
                tokens = tokens.len(),
                text_len = text.len(),
                "segment decoded"
            );
            full_text.push_str(&text);
        }

        let result = full_text.trim().to_string();
        info!(text_len = result.len(), "transcription complete");
        Ok(result)
    }
}

fn token_id(tokenizer: &Tokenizer, token: &str) -> Result<u32, WhisperError> {
    tokenizer
        .token_to_id(token)
        .ok_or_else(|| WhisperError::Tokenizer(format!("no token-id for {token}")))
}

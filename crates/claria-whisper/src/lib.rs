//! claria-whisper
//!
//! Local audio transcription using candle (pure Rust).
//! Supports both English-only and multilingual Whisper models.

pub mod error;

use std::path::Path;

use byteorder::{ByteOrder, LittleEndian};
use candle_core::{Device, IndexOp, Tensor};
use candle_nn::{ops::softmax, VarBuilder};
use candle_transformers::models::whisper::{self as m, audio, Config};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;
use tracing::info;

use crate::error::WhisperError;

static MEL_FILTERS_80: &[u8] = include_bytes!("melfilters.bytes");
static MEL_FILTERS_128: &[u8] = include_bytes!("melfilters128.bytes");

/// Regex-like prefix/suffix for language tokens in the Whisper tokenizer.
/// Multilingual models have tokens like `<|en|>`, `<|es|>`, `<|fr|>`, etc.
const LANG_TOKEN_PREFIX: &str = "<|";
const LANG_TOKEN_SUFFIX: &str = "|>";

/// Result of a transcription, including detected language for multilingual models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeResult {
    pub text: String,
    /// Detected language code (e.g. "en", "es"). `None` for English-only models.
    pub language: Option<String>,
}

/// Returns `true` if GPU acceleration is available for inference.
pub fn is_gpu_available() -> bool {
    #[cfg(feature = "metal")]
    {
        return Device::new_metal(0).is_ok();
    }
    #[allow(unreachable_code)]
    false
}

/// Pick the best available compute device.
///
/// With the `metal` feature enabled, tries Apple Metal GPU first.
/// Falls back to CPU if Metal is unavailable or the feature is off.
fn best_device() -> Device {
    #[cfg(feature = "metal")]
    {
        if let Ok(device) = Device::new_metal(0) {
            info!("using Metal GPU for inference");
            return device;
        }
        info!("Metal unavailable, falling back to CPU");
    }
    Device::Cpu
}

/// A loaded Whisper model ready for transcription.
///
/// Load once via [`WhisperModel::load`], then call [`WhisperModel::transcribe`]
/// repeatedly without reloading weights from disk.
pub struct WhisperModel {
    model: m::model::Whisper,
    config: Config,
    tokenizer: Tokenizer,
    mel_filters: Vec<f32>,
    device: Device,
    sot_token: u32,
    eot_token: u32,
    transcribe_token: u32,
    no_timestamps_token: u32,
    no_speech_token: Option<u32>,
    suppress_tokens: Tensor,
    /// Whether this is a multilingual model (has language tokens in tokenizer).
    is_multilingual: bool,
    /// Resolved language token IDs: (code, token_id) pairs.
    language_tokens: Vec<(String, u32)>,
}

impl WhisperModel {
    /// Load a Whisper model from `model_dir`.
    ///
    /// `model_dir` must contain `model.safetensors`, `config.json`, and
    /// `tokenizer.json` (the HuggingFace layout).
    pub fn load(model_dir: &Path) -> Result<Self, WhisperError> {
        let device = best_device();

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

        // Load mel filters (embedded at compile time, selected by num_mel_bins)
        let mel_bytes = match config.num_mel_bins {
            128 => MEL_FILTERS_128,
            _ => MEL_FILTERS_80,
        };
        let mut mel_filters = vec![0f32; mel_bytes.len() / 4];
        LittleEndian::read_f32_into(mel_bytes, &mut mel_filters);

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

        // Detect multilingual capability by discovering all language tokens
        // in the tokenizer vocabulary. Language tokens look like `<|en|>`, `<|es|>`,
        // etc. — the code is a 2-3 letter lowercase string between `<|` and `|>`.
        let mut language_tokens = Vec::new();
        for (token_str, id) in tokenizer.get_vocab(true) {
            if let Some(code) = token_str
                .strip_prefix(LANG_TOKEN_PREFIX)
                .and_then(|s| s.strip_suffix(LANG_TOKEN_SUFFIX))
            {
                let is_lang_code = (2..=3).contains(&code.len())
                    && code.chars().all(|c| c.is_ascii_lowercase());
                if is_lang_code {
                    language_tokens.push((code.to_string(), id));
                }
            }
        }
        language_tokens.sort_by(|(a, _), (b, _)| a.cmp(b));
        let is_multilingual = !language_tokens.is_empty();
        info!(
            is_multilingual,
            languages = language_tokens.len(),
            "model language support"
        );

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
            device,
            sot_token,
            eot_token,
            transcribe_token,
            no_timestamps_token,
            no_speech_token,
            suppress_tokens,
            is_multilingual,
            language_tokens,
        })
    }

    /// Whether this model supports multiple languages.
    pub fn is_multilingual(&self) -> bool {
        self.is_multilingual
    }

    /// Detect the spoken language from the first 30 seconds of audio.
    ///
    /// Returns the language code (e.g. "en", "es") with the highest probability.
    /// Only meaningful for multilingual models; returns `None` for English-only.
    pub fn detect_language(
        &mut self,
        pcm_16khz: &[f32],
    ) -> Result<Option<String>, WhisperError> {
        if !self.is_multilingual || self.language_tokens.is_empty() {
            return Ok(None);
        }

        let device = &self.device;

        // Use at most 30 seconds of audio
        let max_samples = 30 * m::SAMPLE_RATE;
        let samples = if pcm_16khz.len() > max_samples {
            &pcm_16khz[..max_samples]
        } else {
            pcm_16khz
        };

        // Convert to mel spectrogram
        let mel = audio::pcm_to_mel(&self.config, samples, &self.mel_filters);
        let mel_len = mel.len();
        let mel = Tensor::from_vec(
            mel,
            (
                1,
                self.config.num_mel_bins,
                mel_len / self.config.num_mel_bins,
            ),
            device,
        )
        .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        // Take first segment
        let (_, _, content_frames) = mel
            .dims3()
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;
        let segment_size = usize::min(content_frames, m::N_FRAMES);
        let mel_segment = mel
            .narrow(2, 0, segment_size)
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        // Encode audio
        let audio_features = self
            .model
            .encoder
            .forward(&mel_segment, true)
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        // Run decoder with just [SOT] to get language logits
        let sot = Tensor::new(&[self.sot_token], device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        let ys = self
            .model
            .decoder
            .forward(&sot, &audio_features, true)
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        let logits = self
            .model
            .decoder
            .final_linear(
                &ys.i((..1, 0..1))
                    .map_err(|e| WhisperError::Transcription(e.to_string()))?,
            )
            .and_then(|t| t.i(0))
            .and_then(|t| t.i(0))
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        let probs = softmax(&logits, 0)
            .and_then(|s| s.to_vec1::<f32>())
            .map_err(|e| WhisperError::Transcription(e.to_string()))?;

        // Find the language with highest probability
        let detected = self
            .language_tokens
            .iter()
            .map(|(code, token_id)| {
                let prob = probs.get(*token_id as usize).copied().unwrap_or(0.0);
                (code.clone(), prob)
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(code, prob)| {
                info!(language = %code, probability = format!("{prob:.3}"), "language detected");
                code
            });

        Ok(detected)
    }

    /// Transcribe 16 kHz mono f32 PCM audio.
    ///
    /// For multilingual models, pass `language` to force a specific language
    /// (e.g. `Some("en")` or `Some("es")`). Pass `None` to auto-detect.
    /// For English-only models, the `language` parameter is ignored.
    pub fn transcribe(
        &mut self,
        pcm_16khz: &[f32],
        language: Option<&str>,
    ) -> Result<TranscribeResult, WhisperError> {
        let device = self.device.clone();
        let duration_secs = pcm_16khz.len() as f64 / 16000.0;
        info!(
            samples = pcm_16khz.len(),
            duration_secs = format!("{duration_secs:.1}"),
            is_multilingual = self.is_multilingual,
            "transcribing audio"
        );

        // Resolve the language token for multilingual models
        let (lang_token, detected_language) = if self.is_multilingual {
            let lang_code = match language {
                Some(code) => code.to_string(),
                None => self
                    .detect_language(pcm_16khz)?
                    .unwrap_or_else(|| "en".to_string()),
            };

            let token_id = self
                .language_tokens
                .iter()
                .find(|(c, _)| c == &lang_code)
                .map(|(_, id)| *id)
                .ok_or_else(|| {
                    WhisperError::Transcription(format!(
                        "unsupported language: {lang_code}"
                    ))
                })?;

            info!(language = %lang_code, token_id, "using language token");
            (Some(token_id), Some(lang_code))
        } else {
            (None, None)
        };

        // Convert PCM to mel spectrogram
        let mel = audio::pcm_to_mel(&self.config, pcm_16khz, &self.mel_filters);
        let mel_len = mel.len();
        let mel = Tensor::from_vec(
            mel,
            (
                1,
                self.config.num_mel_bins,
                mel_len / self.config.num_mel_bins,
            ),
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

            // Build decoder prompt
            let sample_len = self.config.max_target_positions / 2;
            let mut tokens = vec![self.sot_token];
            if let Some(lt) = lang_token {
                tokens.push(lt);
            }
            tokens.push(self.transcribe_token);
            tokens.push(self.no_timestamps_token);

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
                    .ok_or_else(|| {
                        WhisperError::Transcription("empty logits".into())
                    })?;

                if next_token == self.eot_token
                    || tokens.len() > self.config.max_target_positions
                {
                    break;
                }
                tokens.push(next_token);
            }

            // Check for no-speech
            if let Some(nst) = self.no_speech_token {
                let mut no_speech_prompt = vec![self.sot_token];
                if let Some(lt) = lang_token {
                    no_speech_prompt.push(lt);
                }
                no_speech_prompt.push(self.transcribe_token);
                no_speech_prompt.push(self.no_timestamps_token);

                let first_tokens =
                    Tensor::new(no_speech_prompt.as_slice(), &device)
                        .and_then(|t| t.unsqueeze(0))
                        .map_err(|e| {
                            WhisperError::Transcription(e.to_string())
                        })?;

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
                            .map_err(|e| {
                                WhisperError::Transcription(e.to_string())
                            })?,
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

            let text =
                self.tokenizer.decode(&tokens, true).map_err(|e| {
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
        info!(
            text_len = result.len(),
            language = ?detected_language,
            "transcription complete"
        );
        Ok(TranscribeResult {
            text: result,
            language: detected_language,
        })
    }
}

fn token_id(
    tokenizer: &Tokenizer,
    token: &str,
) -> Result<u32, WhisperError> {
    tokenizer
        .token_to_id(token)
        .ok_or_else(|| WhisperError::Tokenizer(format!("no token-id for {token}")))
}

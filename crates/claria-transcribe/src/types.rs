use serde::{Deserialize, Serialize};

use crate::LanguageMode;

/// Compute backend requested from transcribe.cpp.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceBackend {
    #[default]
    Auto,
    Cpu,
    CpuAccel,
    Metal,
    Vulkan,
    Cuda,
    Rocm,
}

impl InferenceBackend {
    pub(crate) fn to_native(self) -> transcribe_cpp::Backend {
        match self {
            Self::Auto => transcribe_cpp::Backend::Auto,
            Self::Cpu => transcribe_cpp::Backend::Cpu,
            Self::CpuAccel => transcribe_cpp::Backend::CpuAccel,
            Self::Metal => transcribe_cpp::Backend::Metal,
            Self::Vulkan => transcribe_cpp::Backend::Vulkan,
            Self::Cuda => transcribe_cpp::Backend::Cuda,
            Self::Rocm => transcribe_cpp::Backend::Rocm,
        }
    }
}

/// Decoder K/V cache precision.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KvPrecision {
    #[default]
    Auto,
    F32,
    F16,
}

impl KvPrecision {
    pub(crate) fn to_native(self) -> transcribe_cpp::KvType {
        match self {
            Self::Auto => transcribe_cpp::KvType::Auto,
            Self::F32 => transcribe_cpp::KvType::F32,
            Self::F16 => transcribe_cpp::KvType::F16,
        }
    }
}

/// Whisper-family decoding controls exposed by transcribe.cpp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WhisperOptions {
    /// Terms and context used to bias the first decode window.
    #[serde(default)]
    pub initial_prompt: String,
    /// Carry accepted tokens into the next 30-second window.
    #[serde(default = "default_true")]
    pub condition_on_previous_text: bool,
    /// Maximum number of previous tokens carried between windows.
    #[serde(default = "default_previous_context_tokens")]
    pub max_previous_context_tokens: i32,
    /// First decoding temperature. Zero is deterministic greedy decoding.
    #[serde(default)]
    pub temperature: f32,
    /// Increment used by Whisper's fallback loop.
    #[serde(default = "default_temperature_increment")]
    pub temperature_increment: f32,
    #[serde(default = "default_compression_ratio_threshold")]
    pub compression_ratio_threshold: f32,
    #[serde(default = "default_log_probability_threshold")]
    pub log_probability_threshold: f32,
    #[serde(default = "default_no_speech_threshold")]
    pub no_speech_threshold: f32,
    /// Zero uses a non-deterministic seed when sampling is active.
    #[serde(default)]
    pub seed: u32,
}

impl Default for WhisperOptions {
    fn default() -> Self {
        Self {
            initial_prompt: String::new(),
            condition_on_previous_text: true,
            max_previous_context_tokens: default_previous_context_tokens(),
            temperature: 0.0,
            temperature_increment: default_temperature_increment(),
            compression_ratio_threshold: default_compression_ratio_threshold(),
            log_probability_threshold: default_log_probability_threshold(),
            no_speech_threshold: default_no_speech_threshold(),
            seed: 0,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_previous_context_tokens() -> i32 {
    223
}

fn default_temperature_increment() -> f32 {
    0.2
}

fn default_compression_ratio_threshold() -> f32 {
    2.4
}

fn default_log_probability_threshold() -> f32 {
    -1.0
}

fn default_no_speech_threshold() -> f32 {
    0.6
}

/// Options for one local transcribe.cpp inference pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTranscribeOptions {
    pub language: LanguageMode,
    pub backend: InferenceBackend,
    /// transcribe.cpp device registry index; zero asks it to choose.
    pub gpu_device: i32,
    /// CPU worker threads; zero uses the library default.
    pub n_threads: i32,
    pub kv_precision: KvPrecision,
    pub whisper: WhisperOptions,
}

impl Default for LocalTranscribeOptions {
    fn default() -> Self {
        Self {
            language: LanguageMode::English,
            backend: InferenceBackend::Auto,
            gpu_device: 0,
            n_threads: 0,
            kv_precision: KvPrecision::Auto,
            whisper: WhisperOptions::default(),
        }
    }
}

/// Safe run metadata useful for diagnostics and UI status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTranscription {
    pub text: String,
    pub detected_language: Option<String>,
    pub model_architecture: String,
    pub model_variant: String,
    pub backend: String,
    pub timings: InferenceTimings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct InferenceTimings {
    pub load_ms: f32,
    pub mel_ms: f32,
    pub encode_ms: f32,
    pub decode_ms: f32,
}

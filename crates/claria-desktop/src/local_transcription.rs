use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{State, ipc::Channel};

use crate::state::DesktopState;

const SETTINGS_VERSION: u32 = 1;
const DOWNLOAD_BUFFER_BYTES: usize = 128 * 1_024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelId {
    WhisperBaseEnQ8,
    WhisperSmallQ8,
    WhisperTurboQ8,
}

#[derive(Debug, Clone, Copy)]
struct ModelDescriptor {
    id: LocalModelId,
    filename: &'static str,
    label: &'static str,
    description: &'static str,
    quantization: &'static str,
    languages: &'static [&'static str],
    url: &'static str,
    size_bytes: u64,
    sha256: &'static str,
}

const MODEL_CATALOG: &[ModelDescriptor] = &[
    ModelDescriptor {
        id: LocalModelId::WhisperBaseEnQ8,
        filename: "whisper-base.en-Q8_0.gguf",
        label: "Whisper Base English",
        description: "Fast, compact English-only speech model for live memos.",
        quantization: "Q8_0",
        languages: &["en"],
        url: "https://huggingface.co/handy-computer/whisper-base.en-gguf/resolve/main/whisper-base.en-Q8_0.gguf",
        size_bytes: 84_886_208,
        sha256: "3b46ca40bccbf7609c68d88a36d96077a04ca7c87f2060ede06f129fac3e7652",
    },
    ModelDescriptor {
        id: LocalModelId::WhisperSmallQ8,
        filename: "whisper-small-Q8_0.gguf",
        label: "Whisper Small Multilingual",
        description: "Balanced local model with English, Spanish, and 97 more languages.",
        quantization: "Q8_0",
        languages: &["multilingual", "en", "es"],
        url: "https://huggingface.co/handy-computer/whisper-small-gguf/resolve/main/whisper-small-Q8_0.gguf",
        size_bytes: 269_751_136,
        sha256: "9b9c8811bbcc82a7766f0fb0925614bdacb0923b2cc630daeac17108b655b860",
    },
    ModelDescriptor {
        id: LocalModelId::WhisperTurboQ8,
        filename: "whisper-large-v3-turbo-Q8_0.gguf",
        label: "Whisper Large v3 Turbo",
        description: "Highest-quality curated Whisper model for multilingual transcription.",
        quantization: "Q8_0",
        languages: &["multilingual", "en", "es"],
        url: "https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/main/whisper-large-v3-turbo-Q8_0.gguf",
        size_bytes: 886_381_760,
        sha256: "b2e30cc286bc9f3aba4db9099fc7403543497c05ce7100d0d83091ddfd25a183",
    },
];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalBackend {
    #[default]
    Auto,
    Cpu,
    CpuAccel,
    Metal,
    Vulkan,
    Cuda,
    Rocm,
}

impl LocalBackend {
    fn to_engine(self) -> claria_transcribe::InferenceBackend {
        match self {
            Self::Auto => claria_transcribe::InferenceBackend::Auto,
            Self::Cpu => claria_transcribe::InferenceBackend::Cpu,
            Self::CpuAccel => claria_transcribe::InferenceBackend::CpuAccel,
            Self::Metal => claria_transcribe::InferenceBackend::Metal,
            Self::Vulkan => claria_transcribe::InferenceBackend::Vulkan,
            Self::Cuda => claria_transcribe::InferenceBackend::Cuda,
            Self::Rocm => claria_transcribe::InferenceBackend::Rocm,
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Auto,
            Self::Cpu,
            Self::CpuAccel,
            Self::Metal,
            Self::Vulkan,
            Self::Cuda,
            Self::Rocm,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Cpu => "CPU",
            Self::CpuAccel => "CPU + accelerator",
            Self::Metal => "Metal",
            Self::Vulkan => "Vulkan",
            Self::Cuda => "CUDA",
            Self::Rocm => "ROCm",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalKvPrecision {
    #[default]
    Auto,
    F32,
    F16,
}

impl LocalKvPrecision {
    fn to_engine(self) -> claria_transcribe::KvPrecision {
        match self {
            Self::Auto => claria_transcribe::KvPrecision::Auto,
            Self::F32 => claria_transcribe::KvPrecision::F32,
            Self::F16 => claria_transcribe::KvPrecision::F16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
#[serde(default)]
pub struct LocalTranscriptionSettings {
    pub settings_version: u32,
    pub speech_model: LocalModelId,
    pub backend: LocalBackend,
    pub gpu_device: i32,
    pub cpu_threads: i32,
    pub kv_precision: LocalKvPrecision,
    pub initial_prompt: String,
    pub condition_on_previous_text: bool,
    pub max_previous_context_tokens: i32,
    pub temperature: f32,
    pub temperature_increment: f32,
    pub compression_ratio_threshold: f32,
    pub log_probability_threshold: f32,
    pub no_speech_threshold: f32,
    pub seed: u32,
}

impl Default for LocalTranscriptionSettings {
    fn default() -> Self {
        Self {
            settings_version: SETTINGS_VERSION,
            speech_model: LocalModelId::WhisperSmallQ8,
            backend: LocalBackend::Auto,
            gpu_device: 0,
            cpu_threads: 0,
            kv_precision: LocalKvPrecision::Auto,
            initial_prompt: String::new(),
            condition_on_previous_text: true,
            max_previous_context_tokens: 223,
            temperature: 0.0,
            temperature_increment: 0.2,
            compression_ratio_threshold: 2.4,
            log_probability_threshold: -1.0,
            no_speech_threshold: 0.6,
            seed: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LocalModelInfo {
    pub id: LocalModelId,
    pub label: String,
    pub description: String,
    pub filename: String,
    pub quantization: String,
    pub languages: Vec<String>,
    pub download_size_bytes: u64,
    pub downloaded: bool,
    pub model_size_bytes: Option<u64>,
    pub model_path: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LocalBackendInfo {
    pub backend: LocalBackend,
    pub label: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LocalComputeDevice {
    pub name: String,
    pub description: String,
    pub kind: String,
    pub device_type: String,
    pub device_id: Option<String>,
    pub memory_total: u64,
    pub memory_free: u64,
    pub index: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LocalTranscriptionStatus {
    pub runtime_version: String,
    pub settings: LocalTranscriptionSettings,
    pub models: Vec<LocalModelInfo>,
    pub backends: Vec<LocalBackendInfo>,
    pub devices: Vec<LocalComputeDevice>,
    pub legacy_model_bytes: u64,
    pub accelerated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModelDownloadProgress {
    pub model_id: LocalModelId,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TranscribeMemoResult {
    pub text: String,
    pub language: Option<String>,
    pub model_id: LocalModelId,
    pub backend: String,
}

fn descriptor(id: LocalModelId) -> &'static ModelDescriptor {
    match id {
        LocalModelId::WhisperBaseEnQ8 => &MODEL_CATALOG[0],
        LocalModelId::WhisperSmallQ8 => &MODEL_CATALOG[1],
        LocalModelId::WhisperTurboQ8 => &MODEL_CATALOG[2],
    }
}

fn models_root() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or_else(|| "no data directory found".to_string())?;
    Ok(base.join("com.claria.desktop").join("models"))
}

fn engine_dir() -> Result<PathBuf, String> {
    Ok(models_root()?.join("transcribe-cpp"))
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(engine_dir()?.join("settings.json"))
}

fn model_path(id: LocalModelId) -> Result<PathBuf, String> {
    Ok(engine_dir()?.join(descriptor(id).filename))
}

fn load_settings() -> Result<LocalTranscriptionSettings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(LocalTranscriptionSettings::default());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let settings: LocalTranscriptionSettings = serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if settings.settings_version > SETTINGS_VERSION {
        return Err(format!(
            "local transcription settings version {} is newer than this build supports ({SETTINGS_VERSION})",
            settings.settings_version
        ));
    }
    validate_settings(&settings)?;
    Ok(settings)
}

fn save_settings(settings: &LocalTranscriptionSettings) -> Result<(), String> {
    validate_settings(settings)?;
    let dir = engine_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    let path = settings_path()?;
    let temporary = dir.join("settings.json.tmp");
    let mut stamped = settings.clone();
    stamped.settings_version = SETTINGS_VERSION;
    let bytes = serde_json::to_vec_pretty(&stamped)
        .map_err(|error| format!("failed to serialize local transcription settings: {error}"))?;
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
    set_private_permissions(&temporary)?;
    replace_file(&temporary, &path)
}

/// Atomically replace on Unix; Windows requires removing an existing target
/// before `rename` can install the new file.
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|error| {
            format!(
                "failed to remove old {} before replacement: {error}",
                destination.display()
            )
        })?;
    }

    std::fs::rename(temporary, destination).map_err(|error| {
        format!(
            "failed to replace {} with {}: {error}",
            destination.display(),
            temporary.display()
        )
    })
}

fn validate_settings(settings: &LocalTranscriptionSettings) -> Result<(), String> {
    if settings.gpu_device < 0 {
        return Err("GPU device index cannot be negative".to_string());
    }
    if !(0..=256).contains(&settings.cpu_threads) {
        return Err("CPU threads must be between 0 and 256".to_string());
    }
    if settings.initial_prompt.contains('\0') {
        return Err("initial prompt cannot contain a NUL character".to_string());
    }
    if !(0..=448).contains(&settings.max_previous_context_tokens) {
        return Err("previous context tokens must be between 0 and 448".to_string());
    }
    validate_number("temperature", settings.temperature, 0.0, 1.0)?;
    validate_number(
        "temperature increment",
        settings.temperature_increment,
        0.0,
        1.0,
    )?;
    validate_number(
        "compression ratio threshold",
        settings.compression_ratio_threshold,
        0.1,
        100.0,
    )?;
    validate_number(
        "log probability threshold",
        settings.log_probability_threshold,
        -100.0,
        0.0,
    )?;
    validate_number(
        "no-speech threshold",
        settings.no_speech_threshold,
        0.0,
        1.0,
    )?;
    Ok(())
}

fn validate_number(name: &str, value: f32, minimum: f32, maximum: f32) -> Result<(), String> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(())
}

// Windows leaves `path` unread because this function does nothing there — the
// file keeps whatever its folder handed out. That is a real gap, not a lint
// artifact, and #89 closes it with a protected DACL. The allow keeps the gap
// stated rather than disguised as a `_path` rename, and goes away with #89.
#[cfg_attr(not(unix), allow(unused_variables))]
fn set_private_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect {}: {error}", path.display()))?;
    }
    Ok(())
}

fn model_is_downloaded(id: LocalModelId) -> bool {
    let model = descriptor(id);
    model_path(id)
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == model.size_bytes)
}

fn model_infos(settings: &LocalTranscriptionSettings) -> Result<Vec<LocalModelInfo>, String> {
    MODEL_CATALOG
        .iter()
        .map(|model| {
            let path = model_path(model.id)?;
            let metadata = std::fs::metadata(&path).ok();
            let downloaded = metadata
                .as_ref()
                .is_some_and(|value| value.is_file() && value.len() == model.size_bytes);
            Ok(LocalModelInfo {
                id: model.id,
                label: model.label.to_string(),
                description: model.description.to_string(),
                filename: model.filename.to_string(),
                quantization: model.quantization.to_string(),
                languages: model
                    .languages
                    .iter()
                    .map(|language| (*language).to_string())
                    .collect(),
                download_size_bytes: model.size_bytes,
                downloaded,
                model_size_bytes: metadata.map(|value| value.len()),
                model_path: downloaded.then(|| path.to_string_lossy().to_string()),
                active: settings.speech_model == model.id,
            })
        })
        .collect()
}

fn status() -> Result<LocalTranscriptionStatus, String> {
    let settings = load_settings()?;
    let devices: Vec<LocalComputeDevice> = claria_transcribe::devices()
        .into_iter()
        .map(|device| LocalComputeDevice {
            name: device.name,
            description: device.description,
            kind: device.kind,
            device_type: device.device_type,
            device_id: device.device_id,
            memory_total: device.memory_total,
            memory_free: device.memory_free,
            index: device.index.and_then(|index| u64::try_from(index).ok()),
        })
        .collect();
    let accelerated = match settings.backend {
        LocalBackend::Cpu | LocalBackend::CpuAccel => false,
        LocalBackend::Auto => devices
            .iter()
            .any(|device| !matches!(device.kind.as_str(), "cpu" | "accel")),
        backend => claria_transcribe::backend_available(backend.to_engine()),
    };
    let backends = LocalBackend::all()
        .iter()
        .map(|backend| LocalBackendInfo {
            backend: *backend,
            label: backend.label().to_string(),
            available: claria_transcribe::backend_available(backend.to_engine()),
        })
        .collect();

    Ok(LocalTranscriptionStatus {
        runtime_version: claria_transcribe::runtime_version(),
        models: model_infos(&settings)?,
        settings,
        backends,
        devices,
        legacy_model_bytes: legacy_model_bytes()?,
        accelerated,
    })
}

fn legacy_model_bytes() -> Result<u64, String> {
    let root = models_root()?;
    let mut total = 0_u64;
    for name in [
        "whisper-base-en",
        "whisper-small",
        "whisper-large-v3-turbo",
        "whisper-medium",
        "active-whisper-model.txt",
    ] {
        total = total.saturating_add(path_size(&root.join(name))?);
    }
    Ok(total)
}

fn path_size(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("failed to list {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        total = total.saturating_add(path_size(&entry.path())?);
    }
    Ok(total)
}

fn clear_transcriber(engine: &Arc<Mutex<claria_transcribe::LocalTranscriber>>) {
    match engine.lock() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => {
            tracing::warn!("local transcriber lock was poisoned; clearing cached model");
            poisoned.into_inner().clear();
        }
    }
}

async fn blocking<T>(
    context: &'static str,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("{context} task failed: {error}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_local_transcription_status() -> Result<LocalTranscriptionStatus, String> {
    blocking("local transcription status", status).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_local_transcription_settings(
    state: State<'_, DesktopState>,
    settings: LocalTranscriptionSettings,
) -> Result<LocalTranscriptionStatus, String> {
    let engine = Arc::clone(&state.local_transcriber);
    blocking("save local transcription settings", move || {
        save_settings(&settings)?;
        clear_transcriber(&engine);
        status()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn download_local_model(
    state: State<'_, DesktopState>,
    model_id: LocalModelId,
    on_progress: Channel<ModelDownloadProgress>,
) -> Result<LocalTranscriptionStatus, String> {
    let model = *descriptor(model_id);
    let destination = model_path(model_id)?;
    if model_is_downloaded(model_id) {
        return blocking("local transcription status", status).await;
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    let temporary = destination.with_extension("gguf.download");
    tokio::task::spawn_blocking(move || {
        download_model_file(model, &temporary, &destination, &on_progress)
    })
    .await
    .map_err(|error| format!("model download task failed: {error}"))??;

    let engine = Arc::clone(&state.local_transcriber);
    blocking("finalize local model download", move || {
        let mut settings = load_settings()?;
        if !model_is_downloaded(settings.speech_model) {
            settings.speech_model = model.id;
            save_settings(&settings)?;
        }
        clear_transcriber(&engine);
        status()
    })
    .await
}

fn download_model_file(
    model: ModelDescriptor,
    temporary: &Path,
    destination: &Path,
    on_progress: &Channel<ModelDownloadProgress>,
) -> Result<(), String> {
    tracing::info!(model = ?model.id, url = model.url, "downloading local GGUF model");
    let result = (|| -> Result<(), String> {
        let response = claria_desktop::http::ureq_agent(None)
            .get(model.url)
            .header("User-Agent", "claria-desktop")
            .call()
            .map_err(|error| format!("model download failed: {error}"))?;
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(temporary)
            .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
        set_private_permissions(temporary)?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0_u64;
        let mut buffer = vec![0_u8; DOWNLOAD_BUFFER_BYTES];
        send_download_progress(on_progress, model.id, 0, model.size_bytes);

        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("failed while downloading {}: {error}", model.filename))?;
            if read == 0 {
                break;
            }
            let next_size = downloaded.saturating_add(read as u64);
            if next_size > model.size_bytes {
                return Err(format!(
                    "download for {} exceeded the expected {} bytes",
                    model.filename, model.size_bytes
                ));
            }
            file.write_all(&buffer[..read])
                .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
            hasher.update(&buffer[..read]);
            downloaded = next_size;
            send_download_progress(on_progress, model.id, downloaded, model.size_bytes);
        }
        file.sync_all()
            .map_err(|error| format!("failed to flush {}: {error}", temporary.display()))?;

        if downloaded != model.size_bytes {
            return Err(format!(
                "downloaded {} bytes for {}, expected {}",
                downloaded, model.filename, model.size_bytes
            ));
        }
        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != model.sha256 {
            return Err(format!(
                "download checksum mismatch for {} (expected {}, got {})",
                model.filename, model.sha256, actual_hash
            ));
        }
        replace_file(temporary, destination)?;
        tracing::info!(model = ?model.id, bytes = downloaded, "local GGUF model downloaded");
        Ok(())
    })();

    if result.is_err()
        && temporary.exists()
        && let Err(error) = std::fs::remove_file(temporary)
    {
        tracing::warn!(path = %temporary.display(), error = %error, "failed to remove partial model download");
    }
    result
}

fn send_download_progress(
    channel: &Channel<ModelDownloadProgress>,
    model_id: LocalModelId,
    downloaded_bytes: u64,
    total_bytes: u64,
) {
    if let Err(error) = channel.send(ModelDownloadProgress {
        model_id,
        downloaded_bytes,
        total_bytes,
    }) {
        tracing::debug!(error = %error, "model download progress receiver closed");
    }
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(
    state: State<'_, DesktopState>,
    model_id: LocalModelId,
) -> Result<LocalTranscriptionStatus, String> {
    let engine = Arc::clone(&state.local_transcriber);
    blocking("delete local model", move || {
        // Release native mappings before removal; Windows will not unlink a
        // GGUF while the runtime still has the file open.
        clear_transcriber(&engine);
        let path = model_path(model_id)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|error| format!("failed to remove {}: {error}", path.display()))?;
            tracing::info!(model = ?model_id, "local GGUF model removed");
        }
        status()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_legacy_transcription_models() -> Result<LocalTranscriptionStatus, String> {
    blocking("delete legacy transcription models", || {
        let root = models_root()?;
        for name in [
            "whisper-base-en",
            "whisper-small",
            "whisper-large-v3-turbo",
            "whisper-medium",
        ] {
            let path = root.join(name);
            if path.exists() {
                std::fs::remove_dir_all(&path)
                    .map_err(|error| format!("failed to remove {}: {error}", path.display()))?;
            }
        }
        let marker = root.join("active-whisper-model.txt");
        if marker.exists() {
            std::fs::remove_file(&marker)
                .map_err(|error| format!("failed to remove {}: {error}", marker.display()))?;
        }
        tracing::info!("legacy Candle Whisper model files removed");
        status()
    })
    .await
}

fn inference_options(
    settings: &LocalTranscriptionSettings,
    language: claria_transcribe::LanguageMode,
) -> claria_transcribe::LocalTranscribeOptions {
    claria_transcribe::LocalTranscribeOptions {
        language,
        backend: settings.backend.to_engine(),
        gpu_device: settings.gpu_device,
        n_threads: settings.cpu_threads,
        kv_precision: settings.kv_precision.to_engine(),
        whisper: claria_transcribe::WhisperOptions {
            initial_prompt: settings.initial_prompt.clone(),
            condition_on_previous_text: settings.condition_on_previous_text,
            max_previous_context_tokens: settings.max_previous_context_tokens,
            temperature: settings.temperature,
            temperature_increment: settings.temperature_increment,
            compression_ratio_threshold: settings.compression_ratio_threshold,
            log_probability_threshold: settings.log_probability_threshold,
            no_speech_threshold: settings.no_speech_threshold,
            seed: settings.seed,
        },
    }
}

fn selected_model(
    settings: &LocalTranscriptionSettings,
) -> Result<(LocalModelId, PathBuf), String> {
    let id = settings.speech_model;
    let model = descriptor(id);
    let path = model_path(id)?;
    if !model_is_downloaded(id) {
        return Err(format!(
            "Local model '{}' is not downloaded. Install it in Preferences first.",
            model.label
        ));
    }
    Ok((id, path))
}

fn with_transcriber<T>(
    engine: &Arc<Mutex<claria_transcribe::LocalTranscriber>>,
    operation: impl FnOnce(&mut claria_transcribe::LocalTranscriber) -> Result<T, String>,
) -> Result<T, String> {
    match engine.lock() {
        Ok(mut guard) => operation(&mut guard),
        Err(poisoned) => {
            tracing::warn!("local transcriber lock was poisoned; recovering");
            let mut guard = poisoned.into_inner();
            guard.clear();
            operation(&mut guard)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn transcribe_memo(
    state: State<'_, DesktopState>,
    audio_pcm_base64: String,
) -> Result<TranscribeMemoResult, String> {
    let pcm_bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_pcm_base64)
        .map_err(|error| format!("base64 decode failed: {error}"))?;
    if pcm_bytes.len() % 4 != 0 {
        return Err("PCM data length is not a multiple of four bytes".to_string());
    }
    let pcm_samples: Vec<f32> = pcm_bytes
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    if pcm_samples.is_empty() {
        return Err("memo contains no audio samples".to_string());
    }

    let settings = load_settings()?;
    // Record Memo remains independent from the AWS import defaults. The
    // English-only model is fixed to English; multilingual Whisper models use
    // transcribe.cpp language detection, matching the old on-device behavior.
    let language = match settings.speech_model {
        LocalModelId::WhisperBaseEnQ8 => claria_transcribe::LanguageMode::English,
        LocalModelId::WhisperSmallQ8 | LocalModelId::WhisperTurboQ8 => {
            claria_transcribe::LanguageMode::Mixed
        }
    };
    let (model_id, model_path) = selected_model(&settings)?;
    let options = inference_options(&settings, language);
    let engine = Arc::clone(&state.local_transcriber);
    let result = tokio::task::spawn_blocking(move || {
        with_transcriber(&engine, |transcriber| {
            transcriber
                .transcribe_pcm(&model_path, &pcm_samples, &options)
                .map_err(|error| error.to_string())
        })
    })
    .await
    .map_err(|error| format!("local transcription task failed: {error}"))??;

    let language = result.detected_language.or(Some(
        match language {
            claria_transcribe::LanguageMode::English => "en",
            claria_transcribe::LanguageMode::Spanish => "es",
            claria_transcribe::LanguageMode::Mixed => "mixed",
        }
        .to_string(),
    ));

    Ok(TranscribeMemoResult {
        text: result.text,
        language,
        model_id,
        backend: result.backend,
    })
}

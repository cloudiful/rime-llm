use std::{
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

fn default_bind_addr() -> SocketAddr {
    "127.0.0.1:32123"
        .parse()
        .expect("default bind address is valid")
}

fn default_model_dir() -> PathBuf {
    let current = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if current.join("rime-llm").is_dir() {
        current.join("rime-llm").join("models")
    } else {
        current.join("models")
    }
}

fn default_dictionary_root() -> PathBuf {
    let current = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidates = vec![
        current.join("data/rime-ice"),
        current.join("rime-llm/data/rime-ice"),
    ];
    if let Ok(executable) = env::current_exe() {
        if let Some(project_root) = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            candidates.push(project_root.join("data/rime-ice"));
        }
    }

    candidates
        .into_iter()
        .find(|path| path.join("rime_ice.dict.yaml").is_file())
        .unwrap_or_else(|| current.join("data/rime-ice"))
}

fn default_model_repo() -> String {
    "unsloth/Qwen3-0.6B-GGUF".to_string()
}

fn default_model_file() -> String {
    "Qwen3-0.6B-Q4_K_M.gguf".to_string()
}

fn default_tokenizer_repo() -> String {
    "Qwen/Qwen3-0.6B".to_string()
}

fn default_device() -> DevicePreference {
    DevicePreference::Metal
}

fn default_max_candidates() -> usize {
    16
}

fn default_max_wait_ms() -> u64 {
    15_000
}

fn default_context_window() -> usize {
    4096
}

fn default_max_context_chars() -> usize {
    256
}

fn default_download_timeout_secs() -> u64 {
    3600
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DevicePreference {
    Metal,
    Cpu,
}

impl Default for DevicePreference {
    fn default() -> Self {
        default_device()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub bind_addr: SocketAddr,
    pub model_dir: PathBuf,
    pub model_repo: String,
    pub model_file: String,
    pub tokenizer_repo: String,
    pub device: DevicePreference,
    pub context_window: usize,
    pub max_candidates: usize,
    pub max_wait_ms: u64,
    pub max_context_chars: usize,
    pub download_timeout_secs: u64,
    pub dictionary_root: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            model_dir: default_model_dir(),
            model_repo: default_model_repo(),
            model_file: default_model_file(),
            tokenizer_repo: default_tokenizer_repo(),
            device: default_device(),
            context_window: default_context_window(),
            max_candidates: default_max_candidates(),
            max_wait_ms: default_max_wait_ms(),
            max_context_chars: default_max_context_chars(),
            download_timeout_secs: default_download_timeout_secs(),
            dictionary_root: default_dictionary_root(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

impl Settings {
    pub fn load() -> Result<Self, ConfigError> {
        let path = env::var_os("RIME_LLM_CONFIG")
            .map(PathBuf::from)
            .or_else(default_config_path);
        let mut settings = match path {
            Some(path) if path.exists() => {
                let contents = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
                    path: path.clone(),
                    source,
                })?;
                toml::from_str(&contents).map_err(|source| ConfigError::Parse { path, source })?
            }
            Some(path) if env::var_os("RIME_LLM_CONFIG").is_some() => {
                return Err(ConfigError::Invalid(format!(
                    "RIME_LLM_CONFIG does not exist: {}",
                    path.display()
                )))
            }
            _ => Self::default(),
        };

        settings.apply_environment()?;
        settings.normalize_and_validate()
    }

    pub fn model_path(&self) -> PathBuf {
        self.model_dir.join(&self.model_file)
    }

    pub fn model_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}?download=true",
            self.model_repo.trim_matches('/'),
            self.model_file
        )
    }

    fn apply_environment(&mut self) -> Result<(), ConfigError> {
        if let Some(value) = env::var_os("RIME_LLM_BIND_ADDR") {
            self.bind_addr = value.to_string_lossy().parse().map_err(|_| {
                ConfigError::Invalid("RIME_LLM_BIND_ADDR must be a socket address".to_string())
            })?;
        }
        if let Some(value) = env::var_os("RIME_LLM_MODEL_DIR") {
            self.model_dir = PathBuf::from(value);
        }
        if let Some(value) = env::var_os("RIME_LLM_MODEL_REPO") {
            self.model_repo = value.to_string_lossy().into_owned();
        }
        if let Some(value) = env::var_os("RIME_LLM_MODEL_FILE") {
            self.model_file = value.to_string_lossy().into_owned();
        }
        if let Some(value) = env::var_os("RIME_LLM_TOKENIZER_REPO") {
            self.tokenizer_repo = value.to_string_lossy().into_owned();
        }
        if let Some(value) = env::var_os("RIME_LLM_DEVICE") {
            self.device = match value.to_string_lossy().to_ascii_lowercase().as_str() {
                "metal" => DevicePreference::Metal,
                "cpu" => DevicePreference::Cpu,
                other => {
                    return Err(ConfigError::Invalid(format!(
                        "RIME_LLM_DEVICE must be metal or cpu, got {other}"
                    )))
                }
            };
        }
        if let Some(value) = env::var_os("RIME_LLM_CONTEXT_WINDOW") {
            self.context_window = parse_env_number("RIME_LLM_CONTEXT_WINDOW", &value)?;
        }
        if let Some(value) = env::var_os("RIME_LLM_MAX_CANDIDATES") {
            self.max_candidates = parse_env_number("RIME_LLM_MAX_CANDIDATES", &value)?;
        }
        if let Some(value) = env::var_os("RIME_LLM_MAX_WAIT_MS") {
            self.max_wait_ms = parse_env_number("RIME_LLM_MAX_WAIT_MS", &value)?;
        }
        if let Some(value) = env::var_os("RIME_LLM_MAX_CONTEXT_CHARS") {
            self.max_context_chars = parse_env_number("RIME_LLM_MAX_CONTEXT_CHARS", &value)?;
        }
        if let Some(value) = env::var_os("RIME_LLM_DOWNLOAD_TIMEOUT_SECS") {
            self.download_timeout_secs =
                parse_env_number("RIME_LLM_DOWNLOAD_TIMEOUT_SECS", &value)?;
        }
        if let Some(value) = env::var_os("RIME_LLM_DICTIONARY_ROOT") {
            self.dictionary_root = PathBuf::from(value);
        }
        Ok(())
    }

    fn normalize_and_validate(mut self) -> Result<Self, ConfigError> {
        self.model_repo = self.model_repo.trim().trim_matches('/').to_string();
        self.model_file = self.model_file.trim().to_string();
        self.tokenizer_repo = self.tokenizer_repo.trim().trim_matches('/').to_string();
        if self.model_repo.is_empty() || self.model_file.is_empty() {
            return Err(ConfigError::Invalid(
                "model_repo and model_file must not be empty".to_string(),
            ));
        }
        if self.model_file.contains('/') || self.model_file.contains("..") {
            return Err(ConfigError::Invalid(
                "model_file must be a file name inside model_dir".to_string(),
            ));
        }
        if self.context_window < 128 {
            return Err(ConfigError::Invalid(
                "context_window must be at least 128".to_string(),
            ));
        }
        if self.max_candidates == 0 || self.max_candidates > 64 {
            return Err(ConfigError::Invalid(
                "max_candidates must be between 1 and 64".to_string(),
            ));
        }
        if self.max_wait_ms == 0 || self.max_wait_ms > 60_000 {
            return Err(ConfigError::Invalid(
                "max_wait_ms must be between 1 and 60000".to_string(),
            ));
        }
        if self.max_context_chars == 0 || self.max_context_chars > 4096 {
            return Err(ConfigError::Invalid(
                "max_context_chars must be between 1 and 4096".to_string(),
            ));
        }
        if self.download_timeout_secs == 0 {
            return Err(ConfigError::Invalid(
                "download_timeout_secs must be positive".to_string(),
            ));
        }
        Ok(self)
    }
}

fn default_config_path() -> Option<PathBuf> {
    let current = env::current_dir().ok()?;
    let direct = current.join("config.toml");
    if direct.exists() {
        return Some(direct);
    }
    let nested = current.join("rime-llm").join("config.toml");
    nested.exists().then_some(nested)
}

fn parse_env_number<T>(name: &str, value: &std::ffi::OsStr) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
{
    value
        .to_string_lossy()
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{name} must be a positive integer")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_url_is_stable() {
        let settings = Settings::default();
        assert!(settings.model_url().contains("unsloth/Qwen3-0.6B-GGUF"));
        assert!(settings.model_url().contains("Qwen3-0.6B-Q4_K_M.gguf"));
    }

    #[test]
    fn model_file_cannot_escape_model_dir() {
        let mut settings = Settings::default();
        settings.model_file = "../model.gguf".into();
        assert!(settings.normalize_and_validate().is_err());
    }

    #[test]
    fn default_dictionary_uses_bundled_data() {
        let settings = Settings::default();
        assert!(settings
            .dictionary_root
            .join("rime_ice.dict.yaml")
            .is_file());
    }
}

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub service_url: String,
    pub dictionary_root: PathBuf,
    pub user_freq_path: PathBuf,
    pub max_candidates: usize,
    pub model_timeout_ms: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |key: &str| std::env::var(key).ok();
        let home = env("HOME").unwrap_or_else(|| ".".to_string());
        let dictionary_root = env("RIME_LLM_DICTIONARY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/rime-ice"));
        let user_freq_path = env("RIME_LLM_USER_FREQ")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(&home).join("Library/Application Support/RimeLLM/user_freq.txt")
            });
        Self {
            bind_addr: env("RIME_LLM_DAEMON_BIND").unwrap_or_else(|| "127.0.0.1:32124".to_string()),
            service_url: env("RIME_LLM_SERVICE_URL")
                .unwrap_or_else(|| "http://127.0.0.1:32123".to_string()),
            dictionary_root,
            user_freq_path,
            max_candidates: env("RIME_LLM_MAX_CANDIDATES")
                .and_then(|value| value.parse().ok())
                .unwrap_or(16),
            model_timeout_ms: env("RIME_LLM_MODEL_TIMEOUT_MS")
                .and_then(|value| value.parse().ok())
                .unwrap_or(15_000),
        }
    }
}

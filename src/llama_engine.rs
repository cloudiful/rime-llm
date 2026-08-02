use std::{path::Path, sync::OnceLock};

use anyhow::{anyhow, Context, Result};
use llama_cpp_2::{
    llama_backend::LlamaBackend,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    LogOptions,
};
use tracing::{info, warn};

use crate::{config::DevicePreference, inference, llama_prompt::format_user_prompt};

static LLAMA_BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend> {
    LLAMA_BACKEND
        .get_or_init(|| {
            configure_llama_logs();
            LlamaBackend::init().map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow!(error.clone()))
}

fn configure_llama_logs() {
    let enabled = std::env::var("RIME_LLM_LLAMA_LOG")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    llama_cpp_2::send_logs_to_tracing(LogOptions::default().with_logs_enabled(enabled));
}

pub(crate) struct LlamaEngine {
    backend: &'static LlamaBackend,
    model: LlamaModel,
    context_window: usize,
    device: &'static str,
}

impl LlamaEngine {
    pub(crate) fn load(
        model_path: &Path,
        device_preference: DevicePreference,
        context_window: usize,
    ) -> Result<Self> {
        if context_window == 0 {
            anyhow::bail!("context_window must be positive");
        }
        let backend = backend()?;
        let (model, device) = match device_preference {
            DevicePreference::Cpu => (load_model(backend, model_path, 0)?, "cpu"),
            DevicePreference::Metal if backend.supports_gpu_offload() => {
                match load_model(backend, model_path, u32::MAX) {
                    Ok(model) => (model, "metal"),
                    Err(error) => {
                        warn!(error = %error, "Metal model initialization failed; retrying on CPU");
                        (load_model(backend, model_path, 0)?, "cpu-fallback")
                    }
                }
            }
            DevicePreference::Metal => {
                warn!("Metal support is not compiled; loading the model on CPU");
                (load_model(backend, model_path, 0)?, "cpu")
            }
        };

        info!(device, model = %model_path.display(), "llama.cpp model loaded");
        Ok(Self {
            backend,
            model,
            context_window,
            device,
        })
    }

    pub(crate) fn device(&self) -> &'static str {
        self.device
    }

    pub(crate) fn tokenize_prompt(&self, prompt: &str) -> Result<Vec<u32>> {
        let rendered = format_user_prompt(&self.model, prompt)?;
        inference::tokenize(&self.model, &rendered, AddBos::Never)
    }

    pub(crate) fn tokenize_candidate(&self, text: &str) -> Result<Vec<u32>> {
        inference::tokenize(&self.model, text, AddBos::Never)
    }

    pub(crate) fn logits_at_positions(
        &self,
        tokens: &[u32],
        positions: &[usize],
    ) -> Result<Vec<Vec<f32>>> {
        inference::logits_at_positions(
            &self.model,
            self.backend,
            self.context_window,
            tokens,
            positions,
        )
    }

    pub(crate) fn generate(&self, prompt: &str, max_tokens: usize) -> Result<String> {
        let rendered = format_user_prompt(&self.model, prompt)?;
        let tokens = inference::tokenize(&self.model, &rendered, AddBos::Never)?;
        inference::generate(
            &self.model,
            self.backend,
            self.context_window,
            &tokens,
            max_tokens,
        )
    }
}

fn load_model(backend: &LlamaBackend, model_path: &Path, n_gpu_layers: u32) -> Result<LlamaModel> {
    let params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
    LlamaModel::load_from_file(backend, model_path, &params)
        .with_context(|| format!("load GGUF model {}", model_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a local GGUF path in RIME_LLM_PROBE_MODEL"]
    fn local_model_smoke_covers_tokenizer_logits_and_generation() -> Result<()> {
        let model_path = std::env::var_os("RIME_LLM_PROBE_MODEL")
            .context("RIME_LLM_PROBE_MODEL is required for the smoke test")?;
        let device = match std::env::var("RIME_LLM_PROBE_DEVICE")
            .unwrap_or_else(|_| "cpu".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "metal" => DevicePreference::Metal,
            _ => DevicePreference::Cpu,
        };
        let engine = LlamaEngine::load(Path::new(&model_path), device, 512)?;
        let tokens = engine.tokenize_prompt("只输出中文候选。拼音：buru")?;
        assert_eq!(
            tokens,
            vec![
                151644, 872, 198, 91680, 66017, 104811, 111015, 1773, 116256, 5122, 11240, 84,
                151645, 198, 151644, 77091, 198, 151667, 271, 151668, 271,
            ]
        );
        let logits = engine.logits_at_positions(&tokens, &[tokens.len() - 1])?;
        assert_eq!(logits.len(), 1);
        assert_eq!(logits[0].len(), 151_936);
        assert!(!logits[0].is_empty());
        let output = engine.generate("预测接下来最可能的中文词语。", 4)?;
        println!(
            "device={} tokens={tokens:?} output={output:?}",
            engine.device()
        );
        Ok(())
    }
}

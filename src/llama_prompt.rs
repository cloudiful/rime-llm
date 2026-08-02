use anyhow::{Context, Result};
use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};

const QWEN3_NO_THINKING_SUFFIX: &str = "<|im_start|>assistant\n<think>\n\n</think>\n\n";

/// Apply the chat template stored in the GGUF model.
///
/// Qwen3's template supports `enable_thinking`, but llama.cpp's Rust binding
/// does not expose template variables. Applying the template without an
/// assistant turn and adding the documented empty thinking block gives the
/// same prompt as `enable_thinking(false)`.
pub(crate) fn format_user_prompt(model: &LlamaModel, prompt: &str) -> Result<String> {
    let template = model
        .chat_template(None)
        .context("read chat template from GGUF")?;
    let message = LlamaChatMessage::new("user".to_string(), prompt.to_string())
        .context("build user chat message")?;
    let messages = [message];

    if template
        .to_str()
        .context("decode GGUF chat template")?
        .contains("enable_thinking")
    {
        let mut rendered = model
            .apply_chat_template(&template, &messages, false)
            .context("apply Qwen3 chat template")?;
        rendered.push_str(QWEN3_NO_THINKING_SUFFIX);
        Ok(rendered)
    } else {
        model
            .apply_chat_template(&template, &messages, true)
            .context("apply GGUF chat template")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen3_suffix_disables_thinking() {
        assert_eq!(
            QWEN3_NO_THINKING_SUFFIX,
            "<|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
    }
}

use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result};
use mistralrs::{GgufModelBuilder, Model};
use tracing::{debug, info, warn};

use crate::{
    api::CandidatePath,
    config::{DevicePreference, Settings},
    download::ensure_model,
    inference::{append_candidate, raw_logits_for_tokens, tokenize_candidate, tokenize_prompt},
    lexicon::Lexicon,
    scoring::{rank_paths, score_first_token, score_sequence_normalized, ScoredPath},
};

/// Weight assigned to the native-side `base_score` when blending with the
/// model log-probability. `0.0` makes the model the only signal; `1.0` makes
/// the native prior dominate.
const BASE_SCORE_WEIGHT: f32 = 0.3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelCandidate {
    pub id: String,
    pub text: String,
    pub preedit: String,
    pub consumedkeys: usize,
    pub score: f32,
    #[serde(rename = "type")]
    pub kind: String,
}

pub struct ModelRuntime {
    pub(crate) model: Arc<Model>,
    pub(crate) lexicon: Arc<Lexicon>,
    pub device: String,
    pub model_file: String,
    pub(crate) context_window: usize,
}

impl ModelRuntime {
    pub async fn load(settings: &Settings) -> Result<Self> {
        let model_path = ensure_model(settings).await?;
        let lexicon = Arc::new(Lexicon::load(&settings.dictionary_root));

        let (model, device) = match settings.device {
            DevicePreference::Cpu => (build_model(settings, &model_path, true).await?, "cpu"),
            DevicePreference::Metal => load_metal_or_cpu(settings, &model_path).await?,
        };

        info!(device, model = %settings.model_file, "local model loaded");
        Ok(Self {
            model: Arc::new(model),
            lexicon,
            device: device.to_string(),
            model_file: settings.model_file.clone(),
            context_window: settings.context_window,
        })
    }

    pub async fn candidates(
        &self,
        context: &[String],
        input: &str,
        paths: &[CandidatePath],
        max_candidates: usize,
    ) -> Result<Vec<ModelCandidate>> {
        if paths.is_empty() {
            debug!("no candidate paths supplied; returning empty result");
            return Ok(Vec::new());
        }

        let started = Instant::now();
        let prompt = build_prompt(context, input, self.context_window);
        let prompt_tokens = tokenize_prompt(&self.model, &prompt).await?;
        let prompt_logits = raw_logits_for_tokens(&self.model, &prompt_tokens).await?;
        let first_logits = prompt_logits.last().context("prompt returned no logits")?;

        let mut tokenized_paths: Vec<(CandidatePath, Vec<u32>)> = Vec::with_capacity(paths.len());
        for path in paths {
            if path.text.is_empty() {
                continue;
            }
            match tokenize_candidate(&self.model, &path.text).await {
                Ok(token_ids) if !token_ids.is_empty() => {
                    tokenized_paths.push((path.clone(), token_ids));
                }
                Ok(_) => {}
                Err(error) => {
                    debug!(error = %error, path = %path.id, "skipping untokenizable path");
                }
            }
        }
        if tokenized_paths.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored_paths = Vec::with_capacity(tokenized_paths.len());
        for (path, token_ids) in tokenized_paths {
            let model_score = if token_ids.len() == 1 {
                score_first_token(&token_ids, first_logits)
                    .context("candidate first token is outside model vocabulary")?
            } else {
                let sequence = append_candidate(&prompt_tokens, &token_ids);
                let logits = raw_logits_for_tokens(&self.model, &sequence).await?;
                score_sequence_normalized(&logits, prompt_tokens.len(), &token_ids)
                    .context("candidate sequence returned insufficient logits")?
            };
            scored_paths.push(ScoredPath {
                id: path.id,
                text: path.text,
                preedit: path.preedit,
                consumed_keys: path.consumedkeys,
                base_score: path.base_score,
                model_score,
            });
        }

        let result = rank_paths(scored_paths, max_candidates, BASE_SCORE_WEIGHT)
            .into_iter()
            .map(|scored| candidate_from_path(&scored))
            .collect::<Vec<_>>();
        debug!(
            candidates = result.len(),
            texts = ?result.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "generated model candidates from native paths"
        );
        Ok(result)
    }
}

#[cfg(feature = "metal")]
async fn load_metal_or_cpu(
    settings: &Settings,
    model_path: &std::path::Path,
) -> Result<(Model, &'static str)> {
    match build_model(settings, model_path, false).await {
        Ok(model) => Ok((model, "metal")),
        Err(metal_error) => {
            warn!(error = %metal_error, "Metal model initialization failed; retrying on CPU");
            Ok((
                build_model(settings, model_path, true).await?,
                "cpu-fallback",
            ))
        }
    }
}

#[cfg(not(feature = "metal"))]
async fn load_metal_or_cpu(
    settings: &Settings,
    model_path: &std::path::Path,
) -> Result<(Model, &'static str)> {
    warn!("Metal support is not compiled; loading the model on CPU");
    Ok((build_model(settings, model_path, true).await?, "cpu"))
}

async fn build_model(
    settings: &Settings,
    model_path: &std::path::Path,
    force_cpu: bool,
) -> Result<Model> {
    let model_dir = model_path
        .parent()
        .context("model path has no parent directory")?;
    let model_file = model_path
        .file_name()
        .context("model path has no file name")?
        .to_string_lossy()
        .into_owned();
    let builder = GgufModelBuilder::new(model_dir.to_string_lossy(), vec![model_file])
        .with_tok_model_id(settings.tokenizer_repo.clone())
        .with_max_num_seqs(1)
        .with_prefix_cache_n(None);
    let builder = if force_cpu {
        builder.with_force_cpu()
    } else {
        builder
    };
    builder.build().await.context("build mistral.rs GGUF model")
}

fn build_prompt(context: &[String], input: &str, context_window: usize) -> String {
    let context_budget = context_window
        .saturating_sub(input.chars().count().saturating_add(64))
        .max(16);
    let context = context
        .join("")
        .chars()
        .rev()
        .take(context_budget)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if context.is_empty() {
        format!(
            "请根据拼音输入判断候选中文短语的自然程度。只输出中文，不要解释、标点或拼音。拼音：{input}"
        )
    } else {
        format!(
            "请根据前文和拼音输入判断候选中文短语的自然程度。只输出中文，不要解释、标点或拼音。前文：{context}\n拼音：{input}"
        )
    }
}

fn candidate_from_path(scored: &ScoredPath) -> ModelCandidate {
    ModelCandidate {
        id: scored.id.clone(),
        text: scored.text.clone(),
        preedit: scored.preedit.clone(),
        consumedkeys: scored.consumed_keys,
        score: scored.final_score(BASE_SCORE_WEIGHT),
        kind: "llm_phrase".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_only_explicit_context_and_input() {
        let prompt = build_prompt(&["完全".into()], "buru", 4096);
        assert!(prompt.contains("完全"));
        assert!(prompt.contains("buru"));
        assert!(!prompt.contains("右侧"));
    }

    #[test]
    fn candidate_preserves_partial_consumption() {
        let scored = ScoredPath {
            id: "n0".into(),
            text: "不".into(),
            preedit: "bu".into(),
            consumed_keys: 2,
            base_score: 10.0,
            model_score: 1.0,
        };
        let candidate = candidate_from_path(&scored);
        assert_eq!(candidate.consumedkeys, 2);
        assert_eq!(candidate.preedit, "bu");
        assert_eq!(candidate.text, "不");
    }
}

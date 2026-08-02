use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result};
use model_protocol::ModelCandidate;
use pinyin_dict::Lexicon;
use tracing::debug;

use crate::{
    api::CandidatePath,
    config::Settings,
    download::ensure_model,
    inference_worker::InferenceWorker,
    llama_engine::LlamaEngine,
    scoring::{rank_paths, score_first_token, score_sequence_normalized, ScoredPath},
};

/// Weight assigned to the dictionary `base_score` when blending with the
/// model log-probability. `0.0` makes the model the only signal; `1.0` makes
/// the dictionary prior dominate.
const BASE_SCORE_WEIGHT: f32 = 0.3;

pub struct ModelRuntime {
    pub(crate) inference: Arc<InferenceWorker>,
    pub(crate) lexicon: Arc<Lexicon>,
    pub device: String,
    pub model_file: String,
    pub(crate) context_window: usize,
}

impl ModelRuntime {
    pub async fn load(settings: &Settings) -> Result<Self> {
        let model_path = ensure_model(settings).await?;
        let device_preference = settings.device;
        let context_window = settings.context_window;
        let engine = tokio::task::spawn_blocking(move || {
            LlamaEngine::load(&model_path, device_preference, context_window)
        })
        .await
        .context("llama.cpp model loading task failed")??;
        let device = engine.device().to_string();
        let lexicon = Arc::new(Lexicon::load(&settings.dictionary_root));

        let inference = InferenceWorker::new(Arc::new(Mutex::new(engine)));
        Ok(Self {
            inference,
            lexicon,
            device,
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

        let prompt = build_prompt(context, input, self.context_window);
        let paths = paths.to_vec();
        let started = Instant::now();
        let result = self
            .inference
            .candidates(prompt, paths, max_candidates)
            .await
            .context("candidate inference task failed")?;

        debug!(
            candidates = result.len(),
            texts = ?result.iter().map(|candidate| candidate.text.as_str()).collect::<Vec<_>>(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "generated model candidates from dictionary paths"
        );
        Ok(result)
    }
}

pub(crate) fn score_candidates(
    engine: &LlamaEngine,
    prompt: &str,
    paths: &[CandidatePath],
    max_candidates: usize,
) -> Result<Vec<ModelCandidate>> {
    let prompt_tokens = engine.tokenize_prompt(prompt)?;
    let prompt_last = prompt_tokens
        .len()
        .checked_sub(1)
        .context("chat prompt returned no tokens")?;
    let first_logits = engine
        .logits_at_positions(&prompt_tokens, &[prompt_last])?
        .into_iter()
        .next()
        .context("prompt returned no logits")?;

    let mut scored_paths = Vec::with_capacity(paths.len());
    for path in paths {
        if path.text.is_empty() {
            continue;
        }
        let token_ids = match engine.tokenize_candidate(&path.text) {
            Ok(token_ids) if !token_ids.is_empty() => token_ids,
            Ok(_) => continue,
            Err(error) => {
                debug!(error = %error, path = %path.id, "skipping untokenizable path");
                continue;
            }
        };

        let model_score = if token_ids.len() == 1 {
            score_first_token(&token_ids, &first_logits)
                .context("candidate first token is outside model vocabulary")?
        } else {
            let mut sequence = Vec::with_capacity(prompt_tokens.len() + token_ids.len());
            sequence.extend_from_slice(&prompt_tokens);
            sequence.extend_from_slice(&token_ids);
            let positions = (prompt_tokens.len() - 1..sequence.len() - 1).collect::<Vec<_>>();
            let logits = engine.logits_at_positions(&sequence, &positions)?;
            score_sequence_normalized(&logits, 1, &token_ids)
                .context("candidate sequence returned insufficient logits")?
        };

        scored_paths.push(ScoredPath {
            id: path.id.clone(),
            text: path.text.clone(),
            preedit: path.preedit.clone(),
            consumed_keys: path.consumedkeys,
            base_score: path.base_score,
            model_score,
        });
    }

    Ok(rank_paths(scored_paths, max_candidates, BASE_SCORE_WEIGHT)
        .into_iter()
        .map(|scored| candidate_from_path(&scored))
        .collect())
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

    #[test]
    #[ignore = "requires a local GGUF path in RIME_LLM_PROBE_MODEL"]
    fn local_candidate_scoring_smoke_returns_ranked_paths() -> Result<()> {
        let model_path = std::env::var_os("RIME_LLM_PROBE_MODEL")
            .context("RIME_LLM_PROBE_MODEL is required for the smoke test")?;
        let engine = LlamaEngine::load(
            std::path::Path::new(&model_path),
            crate::config::DevicePreference::Cpu,
            512,
        )?;
        let paths = vec![
            CandidatePath {
                id: "n0".into(),
                text: "不如".into(),
                preedit: "bu ru".into(),
                consumedkeys: 4,
                base_score: 1.0,
            },
            CandidatePath {
                id: "n1".into(),
                text: "不入".into(),
                preedit: "bu ru".into(),
                consumedkeys: 4,
                base_score: 0.5,
            },
        ];
        let candidates = score_candidates(&engine, "只输出中文。拼音：buru", &paths, 2)?;
        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .all(|candidate| candidate.score.is_finite()));
        Ok(())
    }
}

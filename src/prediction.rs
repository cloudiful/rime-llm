use std::collections::HashSet;

use anyhow::{Context, Result};
pub use model_protocol::{PredictionCandidate, PredictionMode};
use serde::{Deserialize, Serialize};

use crate::model::ModelRuntime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionResult {
    pub candidates: Vec<PredictionCandidate>,
    pub source: String,
}

impl ModelRuntime {
    pub async fn predict(
        &self,
        context: &[String],
        mode: PredictionMode,
        max_candidates: usize,
        max_tokens: usize,
    ) -> Result<PredictionResult> {
        let max_candidates = max_candidates.clamp(1, 16);
        let generated = match mode {
            PredictionMode::Dictionary => Vec::new(),
            PredictionMode::Free | PredictionMode::Hybrid => {
                let max_tokens = max_tokens.clamp(1, 32);
                let prompt = build_prediction_prompt(context, self.context_window);
                let output = self
                    .inference
                    .generate(prompt, max_tokens)
                    .await
                    .context("prediction inference task failed")?;
                parse_generated_candidates(&output, max_candidates)
            }
        };

        let dictionary = if matches!(mode, PredictionMode::Free) {
            Vec::new()
        } else {
            self.lexicon
                .top_entries(max_candidates.saturating_mul(4))
                .into_iter()
                .map(|entry| PredictionCandidate {
                    id: format!("d:{}", entry.text),
                    text: entry.text,
                    score: dictionary_score(entry.prior),
                    kind: "dictionary_prediction".to_string(),
                })
                .collect::<Vec<_>>()
        };

        let candidates = combine_predictions(generated, dictionary, mode, max_candidates);

        Ok(PredictionResult {
            candidates,
            source: match mode {
                PredictionMode::Free => "generation",
                PredictionMode::Dictionary => "dictionary",
                PredictionMode::Hybrid => "hybrid",
            }
            .to_string(),
        })
    }
}

fn combine_predictions(
    generated: Vec<PredictionCandidate>,
    dictionary: Vec<PredictionCandidate>,
    mode: PredictionMode,
    max_candidates: usize,
) -> Vec<PredictionCandidate> {
    match mode {
        PredictionMode::Free => rerank_predictions(generated, max_candidates),
        PredictionMode::Dictionary => rerank_predictions(dictionary, max_candidates),
        PredictionMode::Hybrid => rerank_predictions(
            generated.into_iter().chain(dictionary).collect::<Vec<_>>(),
            max_candidates,
        ),
    }
}

fn build_prediction_prompt(context: &[String], context_window: usize) -> String {
    let context_budget = context_window.saturating_sub(96).max(32);
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
        "根据输入法上下文预测接下来最可能的中文词语。只输出中文候选，每行一个，不要解释、序号、标点、空格或拼音。".to_string()
    } else {
        format!(
            "根据前文预测接下来最可能的中文词语。只输出中文候选，每行一个，不要解释、序号、标点、空格或拼音。\n前文：{context}"
        )
    }
}

pub(crate) fn parse_generated_candidates(
    output: &str,
    max_candidates: usize,
) -> Vec<PredictionCandidate> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for line in output.lines() {
        let text = line
            .trim()
            .trim_start_matches(|character: char| character.is_ascii_digit())
            .trim_start_matches(|character: char| {
                matches!(character, '.' | ')' | '、' | '-' | '*' | ' ')
            })
            .trim();
        if !is_chinese_text(text) || !seen.insert(text.to_string()) {
            continue;
        }
        result.push(PredictionCandidate {
            id: format!("g{}", result.len()),
            text: text.to_string(),
            score: 1.0 / (result.len() as f32 + 1.0),
            kind: "llm_prediction".to_string(),
        });
        if result.len() >= max_candidates {
            break;
        }
    }
    result
}

fn deduplicate_predictions(
    candidates: Vec<PredictionCandidate>,
    max_candidates: usize,
) -> Vec<PredictionCandidate> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        if seen.insert(candidate.text.clone()) {
            result.push(candidate);
        }
        if result.len() >= max_candidates {
            break;
        }
    }
    result
}

fn rerank_predictions(
    candidates: Vec<PredictionCandidate>,
    max_candidates: usize,
) -> Vec<PredictionCandidate> {
    let mut candidates = deduplicate_predictions(candidates, usize::MAX);
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.text.len().cmp(&right.text.len()))
    });
    candidates.truncate(max_candidates);
    candidates
}

fn dictionary_score(prior: f32) -> f32 {
    (prior.max(0.0) + 1.0).ln() * 0.15
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn merge_modes_follow_expected_sources() {
        let generated = vec![
            PredictionCandidate {
                id: "g0".into(),
                text: "苹果".into(),
                score: 1.0,
                kind: "llm_prediction".into(),
            },
            PredictionCandidate {
                id: "g1".into(),
                text: "香蕉".into(),
                score: 0.5,
                kind: "llm_prediction".into(),
            },
        ];
        let dictionary = vec![PredictionCandidate {
            id: "d:香蕉".into(),
            text: "香蕉".into(),
            score: 0.25,
            kind: "dictionary_prediction".into(),
        }];

        let free = combine_predictions(
            generated.clone(),
            dictionary.clone(),
            PredictionMode::Free,
            5,
        );
        assert_eq!(free.len(), 2);

        let dictionary_only = combine_predictions(
            generated.clone(),
            dictionary.clone(),
            PredictionMode::Dictionary,
            5,
        );
        assert_eq!(dictionary_only.len(), 1);
        assert_eq!(dictionary_only[0].text, "香蕉");

        let hybrid = combine_predictions(generated, dictionary, PredictionMode::Hybrid, 5);
        assert_eq!(hybrid.len(), 2);
    }
}

fn is_chinese_text(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|character| {
            let code = character as u32;
            (0x3400..=0x4dbf).contains(&code)
                || (0x4e00..=0x9fff).contains(&code)
                || (0xf900..=0xfaff).contains(&code)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_output_only_keeps_chinese_candidates() {
        let candidates = parse_generated_candidates("1. 苹果\nbanana\n香蕉\n橘子!", 5);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            ["苹果", "香蕉"]
        );
    }

    #[test]
    fn generated_candidates_are_deduplicated_and_limited() {
        let candidates = parse_generated_candidates("苹果\n苹果\n香蕉", 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].text, "苹果");
    }

    #[test]
    fn dictionary_score_is_finite_for_invalid_prior() {
        assert!(dictionary_score(-1.0).is_finite());
    }
}

use crate::lexicon::LexiconEntry;

pub fn score_first_token(token_ids: &[u32], logits: &[f32]) -> Option<f32> {
    let token_id = *token_ids.first()? as usize;
    log_probability(logits, token_id)
}

/// Score all candidate tokens after the prompt. The raw logits row at
/// `prompt_len - 1` predicts the first candidate token.
pub fn score_sequence(
    logits: &[Vec<f32>],
    prompt_len: usize,
    candidate_tokens: &[u32],
) -> Option<f32> {
    if prompt_len == 0 || candidate_tokens.is_empty() {
        return None;
    }
    let mut score = 0.0;
    for (offset, token_id) in candidate_tokens.iter().enumerate() {
        let row = logits.get(prompt_len + offset - 1)?;
        score += log_probability(row, *token_id as usize)?;
    }
    Some(score)
}

/// Average per-token log probability. Divides the sum of log-probabilities by
/// the candidate token count so longer candidates do not dominate purely
/// because they accumulate more terms. Returns `None` for empty candidates.
pub fn score_sequence_normalized(
    logits: &[Vec<f32>],
    prompt_len: usize,
    candidate_tokens: &[u32],
) -> Option<f32> {
    if candidate_tokens.is_empty() {
        return None;
    }
    let total = score_sequence(logits, prompt_len, candidate_tokens)?;
    Some(total / candidate_tokens.len() as f32)
}

fn log_probability(logits: &[f32], token_id: usize) -> Option<f32> {
    let token_logit = *logits.get(token_id)?;
    let max_logit = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !max_logit.is_finite() || !token_logit.is_finite() {
        return None;
    }
    let log_sum_exp = max_logit
        + logits
            .iter()
            .map(|logit| (*logit - max_logit).exp())
            .sum::<f32>()
            .ln();
    Some(token_logit - log_sum_exp)
}

fn prior_score(entry: &LexiconEntry) -> f32 {
    (entry.prior.max(0.0) + 1.0).ln() * 0.15
}

pub fn rank_entries_by_scores(
    entries: Vec<(f32, LexiconEntry)>,
    max_candidates: usize,
) -> Vec<(f32, LexiconEntry)> {
    let mut scored = entries
        .into_iter()
        .map(|(model_score, entry)| (model_score + prior_score(&entry), entry))
        .collect::<Vec<_>>();

    // `sort_by` is stable, so equal scores retain dictionary order.
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.truncate(max_candidates);
    scored
}

/// Native-supplied candidate path with a base prior from the Rime pipeline.
/// `base_score` is on the same scale as `prior_score` so it can be combined
/// additively with the model score.
#[derive(Debug, Clone)]
pub struct ScoredPath {
    pub id: String,
    pub text: String,
    pub preedit: String,
    pub consumed_keys: usize,
    pub base_score: f32,
    pub model_score: f32,
}

impl ScoredPath {
    /// Final ranking score combining the native prior and the per-token
    /// normalized model log-probability.
    pub fn final_score(&self, base_weight: f32) -> f32 {
        let model_weight = 1.0 - base_weight.clamp(0.0, 1.0);
        self.base_score * base_weight + self.model_score * model_weight
    }
}

/// Dedup, sort and truncate a list of `ScoredPath`s. The first occurrence of a
/// given `text` wins, preserving native ordering before sorting by score.
pub fn rank_paths(
    paths: Vec<ScoredPath>,
    max_candidates: usize,
    base_weight: f32,
) -> Vec<ScoredPath> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<ScoredPath> = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.text.clone()) {
            deduped.push(path);
        }
    }
    deduped.sort_by(|left, right| {
        right
            .final_score(base_weight)
            .total_cmp(&left.final_score(base_weight))
            .then_with(|| left.text.chars().count().cmp(&right.text.chars().count()))
    });
    deduped.truncate(max_candidates.max(1));
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str, prior: f32) -> LexiconEntry {
        LexiconEntry {
            text: text.to_string(),
            code: "buru".to_string(),
            preedit: "bu ru".to_string(),
            consumed_keys: 4,
            prior,
        }
    }

    #[test]
    fn contextual_logits_can_overrule_dictionary_prior() {
        let ranked = rank_entries_by_scores(
            vec![
                (5.0, entry("不如", 20_830.0)),
                (0.1, entry("不入", 479_325.0)),
            ],
            2,
        );
        assert_eq!(ranked[0].1.text, "不如");
    }

    #[test]
    fn full_sequence_score_distinguishes_shared_first_token() {
        let rows = vec![vec![0.0, 0.0], vec![0.0, 4.0], vec![0.0, 0.0]];
        let first = score_sequence(&rows, 1, &[0, 1]).unwrap();
        let second = score_sequence(&rows, 1, &[0, 0]).unwrap();
        assert!(first > second);
    }

    #[test]
    fn first_token_score_is_a_log_probability() {
        let score = score_first_token(&[1], &[0.0, 2.0]).unwrap();
        assert!(score < 0.0);
        assert!(score > -1.0);
    }

    #[test]
    fn equal_scores_keep_original_order_and_respect_limit() {
        let ranked = rank_entries_by_scores(
            vec![
                (1.0, entry("甲", 1.0)),
                (1.0, entry("乙", 1.0)),
                (1.0, entry("丙", 1.0)),
            ],
            2,
        );
        assert_eq!(
            ranked
                .iter()
                .map(|(_, item)| item.text.as_str())
                .collect::<Vec<_>>(),
            ["甲", "乙"]
        );
    }

    #[test]
    fn invalid_sequence_shape_returns_none() {
        assert!(score_sequence(&[], 1, &[0]).is_none());
        assert!(score_sequence(&[vec![0.0]], 0, &[0]).is_none());
        assert!(score_sequence(&[vec![0.0]], 1, &[2]).is_none());
    }

    fn path(id: &str, text: &str, base: f32, model: f32) -> ScoredPath {
        ScoredPath {
            id: id.into(),
            text: text.into(),
            preedit: String::new(),
            consumed_keys: text.chars().count().max(1),
            base_score: base,
            model_score: model,
        }
    }

    #[test]
    fn normalized_score_divides_by_token_count() {
        let rows = vec![vec![0.0, 4.0], vec![0.0, 4.0], vec![0.0, 4.0]];
        let one = score_sequence_normalized(&rows, 1, &[1]).unwrap();
        let two = score_sequence_normalized(&rows, 1, &[1, 1]).unwrap();
        let sum_two = score_sequence(&rows, 1, &[1, 1]).unwrap();
        assert!((one - two).abs() < 1e-6);
        assert!((one - sum_two / 2.0).abs() < 1e-6);
        assert!(score_sequence_normalized(&rows, 1, &[]).is_none());
    }

    #[test]
    fn rank_paths_dedups_by_text_and_respects_limit() {
        let paths = vec![
            path("a", "苹果", 0.0, -0.1),
            path("b", "苹果", 0.5, -1.0),
            path("c", "香蕉", 0.2, -0.5),
            path("d", "橘子", 0.3, -0.2),
        ];
        let ranked = rank_paths(paths, 2, 0.5);
        let texts: Vec<&str> = ranked.iter().map(|p| p.text.as_str()).collect();
        let ids: Vec<&str> = ranked.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(texts, vec!["橘子", "苹果"]);
        assert_eq!(ids, vec!["d", "a"]);
    }

    #[test]
    fn rank_paths_keeps_first_native_occurrence_when_deduping() {
        let paths = vec![
            path("weak", "苹果", 0.0, -0.1),
            path("strong", "苹果", 10.0, -0.1),
        ];
        let ranked = rank_paths(paths, 1, 0.0);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "weak");
    }

    #[test]
    fn rank_paths_keeps_native_order_on_equal_scores() {
        let paths = vec![
            path("a", "甲", 0.0, -0.1),
            path("b", "乙", 0.0, -0.1),
            path("c", "丙", 0.0, -0.1),
        ];
        let ranked = rank_paths(paths, 3, 0.5);
        assert_eq!(
            ranked.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn rank_paths_blends_base_and_model_with_weight() {
        let paths = vec![path("model", "甲", 0.0, 5.0), path("base", "乙", 5.0, 0.0)];
        let heavy_model = rank_paths(paths.clone(), 2, 0.0);
        assert_eq!(heavy_model[0].id, "model");
        let heavy_base = rank_paths(paths, 2, 1.0);
        assert_eq!(heavy_base[0].id, "base");
    }
}

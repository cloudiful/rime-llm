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
}

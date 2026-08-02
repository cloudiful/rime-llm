//! Dictionary candidate lattice: word edges over syllable segmentations,
//! completion reachability, and candidate extraction.

use std::collections::{HashMap, HashSet, VecDeque};

use pinyin_dict::{Lexicon, LexiconEntry, UserFreq};

use crate::syllable::{SyllableEdge, SyllableTable};

pub const DEFAULT_MAX_CANDIDATES: usize = 16;
const MAX_WORD_SYLLABLES: usize = 8;
const MAX_PATHS_PER_START: usize = 64;
const MAX_WORDS_PER_CODE: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Stable across refreshes: `d:` + text, so the model rerank can echo
    /// the same id and the UI can preserve the selection.
    pub id: String,
    pub text: String,
    /// Syllable preedit of the first word, for example `bu ru`.
    pub preedit: String,
    /// Number of input letters consumed by the first word.
    pub consumedkeys: usize,
    /// Dictionary prior plus user frequency boost.
    pub base_score: f32,
    pub kind: String,
}

#[derive(Debug, Clone)]
struct WordEdge {
    start: usize,
    end: usize,
    entry: LexiconEntry,
}

pub struct CandidateEngine<'a> {
    lexicon: &'a Lexicon,
    user_freq: &'a UserFreq,
}

impl<'a> CandidateEngine<'a> {
    pub fn new(lexicon: &'a Lexicon, user_freq: &'a UserFreq) -> Self {
        Self { lexicon, user_freq }
    }

    pub fn candidates(&self, input: &str) -> Vec<Candidate> {
        self.candidates_limited(input, DEFAULT_MAX_CANDIDATES)
    }

    /// Menu candidates are first-segment words that participate in a
    /// complete covering path, ordered by base score. When no complete path
    /// exists, fall back to the words reachable from the longest valid
    /// syllable prefix.
    pub fn candidates_limited(&self, input: &str, max_candidates: usize) -> Vec<Candidate> {
        let max_candidates = max_candidates.max(1);
        let table = SyllableTable::from_set(self.lexicon.syllables());
        let edges = table.edges(input);
        let words = enumerate_word_edges(input, &edges, self.lexicon);
        let letter_count = input.len();
        let mut by_start: HashMap<usize, Vec<&WordEdge>> = HashMap::new();
        for word in &words {
            by_start.entry(word.start).or_default().push(word);
        }

        let mut reach_end = vec![false; letter_count + 1];
        reach_end[letter_count] = true;
        for position in (0..letter_count).rev() {
            reach_end[position] = by_start
                .get(&position)
                .is_some_and(|words| words.iter().any(|word| reach_end[word.end]));
        }
        let complete = reach_end[0];
        let prefix_end = edges.iter().map(|edge| edge.end).max().unwrap_or(0);

        let mut by_text: HashMap<String, Candidate> = HashMap::new();
        if let Some(first_words) = by_start.get(&0) {
            for word in first_words {
                let qualifies = if complete {
                    reach_end[word.end]
                } else {
                    word.end == prefix_end
                };
                if !qualifies {
                    continue;
                }
                let candidate = Candidate {
                    id: format!("d:{}", word.entry.text),
                    text: word.entry.text.clone(),
                    preedit: word.entry.code.clone(),
                    consumedkeys: word.end,
                    base_score: word_score(&word.entry, self.user_freq),
                    kind: "dictionary".to_string(),
                };
                by_text
                    .entry(candidate.text.clone())
                    .and_modify(|existing| {
                        if candidate.base_score > existing.base_score {
                            *existing = candidate.clone();
                        }
                    })
                    .or_insert(candidate);
            }
        }
        let mut candidates = by_text.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .base_score
                .total_cmp(&left.base_score)
                .then_with(|| left.text.chars().count().cmp(&right.text.chars().count()))
                .then_with(|| left.text.cmp(&right.text))
        });
        candidates.truncate(max_candidates);
        candidates
    }
}

fn enumerate_word_edges(input: &str, edges: &[SyllableEdge], lexicon: &Lexicon) -> Vec<WordEdge> {
    let mut by_start: HashMap<usize, Vec<&SyllableEdge>> = HashMap::new();
    for edge in edges {
        by_start.entry(edge.start).or_default().push(edge);
    }

    let bytes = input.as_bytes();
    let mut words = Vec::new();
    let mut starts = by_start.keys().copied().collect::<Vec<_>>();
    starts.sort_unstable();
    for start in starts {
        let mut queue = VecDeque::new();
        queue.push_back((start, Vec::<String>::new()));
        let mut visited = HashSet::new();
        let mut explored = 0usize;
        while let Some((position, syllables)) = queue.pop_front() {
            if explored >= MAX_PATHS_PER_START {
                break;
            }
            explored += 1;
            for entry in lexicon
                .entries_for(&syllables.iter().map(String::as_str).collect::<Vec<_>>())
                .into_iter()
                .take(MAX_WORDS_PER_CODE)
            {
                words.push(WordEdge {
                    start,
                    end: position,
                    entry,
                });
            }
            if syllables.len() >= MAX_WORD_SYLLABLES {
                continue;
            }
            let mut next_positions = vec![position];
            if position < bytes.len() && bytes[position] == b'\'' {
                next_positions.push(position + 1);
            }
            for next_position in next_positions {
                let Some(next_edges) = by_start.get(&next_position) else {
                    continue;
                };
                for edge in next_edges {
                    let mut next = syllables.clone();
                    next.push(edge.syllable.clone());
                    if visited.insert((edge.end, next.join(" "))) {
                        queue.push_back((edge.end, next));
                    }
                }
            }
        }
    }
    words
}

fn word_score(entry: &LexiconEntry, user_freq: &UserFreq) -> f32 {
    entry.prior_score() + user_freq.boost(&entry.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn entry(text: &str, code: &str, prior: f32) -> pinyin_dict::LexiconEntry {
        pinyin_dict::LexiconEntry {
            text: text.to_string(),
            code: code.to_string(),
            preedit: code.to_string(),
            consumed_keys: code.chars().filter(|c| *c != ' ').count(),
            prior,
        }
    }

    fn engine(entries: Vec<pinyin_dict::LexiconEntry>) -> CandidateEngine<'static> {
        let lexicon = Box::leak(Box::new(Lexicon::from_entries(entries)));
        let freq = Box::leak(Box::new(UserFreq::default()));
        CandidateEngine::new(lexicon, freq)
    }

    fn texts(candidates: &[Candidate]) -> Vec<&str> {
        candidates
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect()
    }

    #[test]
    fn ambiguous_input_offers_both_segmentations() {
        let engine = engine(vec![
            entry("先", "xian", 100.0),
            entry("西安", "xi an", 50.0),
        ]);
        let candidates = engine.candidates("xian");
        assert_eq!(texts(&candidates), vec!["先", "西安"]);
        assert_eq!(candidates[1].consumedkeys, 4);
    }

    #[test]
    fn apostrophe_restricts_to_forced_segmentation() {
        let engine = engine(vec![
            entry("先", "xian", 100.0),
            entry("西安", "xi an", 50.0),
        ]);
        let candidates = engine.candidates("xi'an");
        assert_eq!(texts(&candidates), vec!["西安"]);
    }

    #[test]
    fn complete_paths_win_over_prefix_only() {
        let engine = engine(vec![
            entry("不", "bu", 900.0),
            entry("不如", "bu ru", 100.0),
        ]);
        let candidates = engine.candidates("buru");
        assert_eq!(texts(&candidates), vec!["不如"]);
    }

    #[test]
    fn incomplete_input_falls_back_to_longest_prefix() {
        let engine = engine(vec![
            entry("不", "bu", 900.0),
            entry("不如", "bu ru", 100.0),
        ]);
        let candidates = engine.candidates("bunr");
        assert_eq!(texts(&candidates), vec!["不"]);
        assert_eq!(candidates[0].consumedkeys, 2);
    }

    #[test]
    fn polyphonic_words_are_found_under_each_code() {
        let engine = engine(vec![
            entry("长", "chang", 100.0),
            entry("长", "zhang", 60.0),
        ]);
        assert_eq!(texts(&engine.candidates("chang")), vec!["长"]);
        assert_eq!(texts(&engine.candidates("zhang")), vec!["长"]);
    }

    #[test]
    fn long_words_span_many_syllables() {
        let engine = engine(vec![
            entry("阿部相模肩灯鱼", "a bu xiang mo jian deng yu", 200.0),
            entry("阿", "a", 1.0),
        ]);
        let candidates = engine.candidates("abuxiangmojiandengyu");
        assert_eq!(texts(&candidates), vec!["阿部相模肩灯鱼"]);
        assert_eq!(candidates[0].consumedkeys, 20);
    }

    #[test]
    fn homophone_cap_limits_candidates_per_code() {
        let mut entries = Vec::new();
        for index in 0..64 {
            entries.push(entry(&format!("字{index}"), "shi", (64 - index) as f32));
        }
        let engine = engine(entries);
        let candidates = engine.candidates("shi");
        assert_eq!(candidates.len(), DEFAULT_MAX_CANDIDATES);
    }

    #[test]
    fn ordering_is_stable_and_prior_driven() {
        let engine = engine(vec![
            entry("甲", "jia", 10.0),
            entry("假", "jia", 5.0),
            entry("加", "jia", 5.0),
        ]);
        let candidates = engine.candidates("jia");
        assert_eq!(texts(&candidates), vec!["甲", "假", "加"]);
    }

    #[test]
    fn user_frequency_boost_reorders_candidates() {
        let lexicon = Box::leak(Box::new(Lexicon::from_entries(vec![
            entry("不如", "bu ru", 100.0),
            entry("不入", "bu ru", 500.0),
        ])));
        let freq = Box::leak(Box::new(UserFreq::default()));
        for _ in 0..20 {
            freq.record("不如");
        }
        let engine = CandidateEngine::new(lexicon, freq);
        let candidates = engine.candidates("buru");
        assert_eq!(texts(&candidates), vec!["不如", "不入"]);
        assert!(candidates[0].base_score > candidates[1].base_score);
    }

    #[test]
    fn single_syllable_words_chain_into_paths() {
        let engine = engine(vec![
            entry("不", "bu", 10.0),
            entry("如", "ru", 10.0),
            entry("不如", "bu ru", 100.0),
        ]);
        let candidates = engine.candidates("buru");
        assert_eq!(texts(&candidates), vec!["不如", "不"]);
        assert_eq!(candidates[1].consumedkeys, 2);
    }

    #[test]
    fn real_dictionary_produces_expected_candidates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/rime-ice");
        let lexicon = Lexicon::load(&root);
        let freq = UserFreq::default();
        let engine = CandidateEngine::new(&lexicon, &freq);

        let buru = engine.candidates("buru");
        assert!(buru.iter().any(|candidate| candidate.text == "不如"));
        assert!(buru
            .iter()
            .find(|candidate| candidate.text == "不如")
            .is_some_and(|candidate| candidate.consumedkeys == 4));

        let prefix = engine.candidates("bunr");
        assert!(prefix.iter().any(|candidate| candidate.text == "不"));
        assert!(prefix.iter().all(|candidate| candidate.consumedkeys == 2));

        let xian = engine.candidates("xian");
        assert!(xian.iter().any(|candidate| candidate.text == "先"));
        assert!(
            engine
                .candidates_limited("xian", 64)
                .iter()
                .any(|candidate| candidate.text == "西安"),
            "xian must also admit the xi an segmentation"
        );

        let separated = engine.candidates_limited("xi'an", 64);
        assert!(
            separated.iter().any(|candidate| candidate.text == "西安"),
            "apostrophe must select the xi an segmentation"
        );
        assert!(
            !separated.iter().any(|candidate| candidate.text == "先"),
            "xian must not cross the apostrophe"
        );
    }
}

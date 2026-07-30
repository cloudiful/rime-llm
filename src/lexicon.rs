use std::{collections::HashMap, fs, path::Path};

use tracing::{debug, warn};

use crate::dictionary_files::dictionary_files;

#[derive(Debug, Clone)]
pub struct LexiconEntry {
    pub text: String,
    pub code: String,
    pub preedit: String,
    pub consumed_keys: usize,
    pub prior: f32,
}

#[derive(Debug, Default)]
pub struct Lexicon {
    by_code: HashMap<String, Vec<LexiconEntry>>,
    pub pinyin_by_word: HashMap<String, String>,
}

impl Lexicon {
    pub fn load(root: &Path) -> Self {
        let mut lexicon = Self::default();
        for path in dictionary_files(root) {
            if let Err(error) = lexicon.load_file(&path) {
                debug!(path = %path.display(), %error, "skipping unreadable Rime dictionary");
            }
        }
        for entries in lexicon.by_code.values_mut() {
            entries.sort_by(|left, right| right.prior.total_cmp(&left.prior));
            entries.dedup_by(|right, left| right.text == left.text);
        }
        if lexicon.by_code.is_empty() {
            warn!(root = %root.display(), "no Rime dictionary entries were loaded");
        }
        lexicon
    }

    pub fn candidates_for(&self, input: &str, max_candidates: usize) -> Vec<LexiconEntry> {
        let code = normalize_code(input);
        if code.is_empty() {
            return Vec::new();
        }

        let mut candidates = Vec::new();
        for length in 1..=code.len() {
            if !code.is_char_boundary(length) {
                continue;
            }
            if let Some(entries) = self.by_code.get(&code[..length]) {
                candidates.extend(entries.iter().take(max_candidates).cloned());
            }
        }
        candidates.sort_by(|left, right| right.prior.total_cmp(&left.prior));
        candidates.dedup_by(|right, left| {
            if right.text == left.text {
                if right.consumed_keys > left.consumed_keys {
                    left.consumed_keys = right.consumed_keys;
                    left.code = right.code.clone();
                }
                true
            } else {
                false
            }
        });
        candidates.truncate(max_candidates);
        candidates
    }

    fn load_file(&mut self, path: &Path) -> Result<(), std::io::Error> {
        if !path.is_file() {
            return Ok(());
        }
        let contents = fs::read_to_string(path)?;
        let mut loaded = 0usize;
        for line in contents.lines() {
            let Some(entry) = parse_entry(line) else {
                continue;
            };
            self.pinyin_by_word
                .entry(entry.text.clone())
                .or_insert_with(|| entry.code.clone());
            let entries = self.by_code.entry(entry.code.clone()).or_default();
            if let Some(existing) = entries.iter_mut().find(|item| item.text == entry.text) {
                if entry.prior > existing.prior {
                    existing.prior = entry.prior;
                }
            } else {
                entries.push(entry);
                loaded += 1;
            }
        }
        debug!(path = %path.display(), entries = loaded, "loaded Rime dictionary");
        Ok(())
    }
}

fn parse_entry(line: &str) -> Option<LexiconEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with("---") {
        return None;
    }
    let mut fields = line.split('\t');
    let text = fields.next()?.trim();
    let preedit = fields.next()?.trim().to_ascii_lowercase();
    let code = normalize_code(&preedit);
    if text.is_empty() || code.is_empty() || !is_chinese_text(text) || !is_pinyin_code(&code) {
        return None;
    }
    let prior = fields
        .next()
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1.0);
    Some(LexiconEntry {
        text: text.to_string(),
        consumed_keys: code.len(),
        code,
        preedit,
        prior,
    })
}

fn normalize_code(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_chinese_text(text: &str) -> bool {
    text.chars().any(|character| {
        let code = character as u32;
        (0x3400..=0x4dbf).contains(&code)
            || (0x4e00..=0x9fff).contains(&code)
            || (0xf900..=0xfaff).contains(&code)
    })
}

fn is_pinyin_code(code: &str) -> bool {
    code.chars().all(|character| character.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_weighted_chinese_entry() {
        let entry = parse_entry("不如\tbu ru\t479325").unwrap();
        assert_eq!(entry.code, "buru");
        assert_eq!(entry.preedit, "bu ru");
        assert_eq!(entry.consumed_keys, 4);
        assert_eq!(entry.prior, 479325.0);
    }

    #[test]
    fn ignores_headers_and_non_chinese_entries() {
        assert!(parse_entry("name: rime_ice").is_none());
        assert!(parse_entry("hello\thello\t10").is_none());
        assert!(parse_entry("检委会\t100").is_none());
    }

    #[test]
    fn loads_bundled_dictionary_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/rime-ice");
        let lexicon = Lexicon::load(&root);
        let candidates = lexicon.candidates_for("buru", 16);

        assert!(candidates.iter().any(|entry| entry.text == "不如"));
        assert!(lexicon.pinyin_by_word.len() > 100_000);
    }
}

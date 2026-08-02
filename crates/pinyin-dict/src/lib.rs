//! Static Rime dictionary loading and pinyin syllable indexing.
//!
//! Entries are indexed by their syllable sequence (for example `bu ru`),
//! not by the raw letter string, so segmentations such as `xi an` and
//! `xian` stay distinct instead of colliding under the joined letters.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use tracing::{debug, warn};

mod user_freq;

pub use user_freq::UserFreq;

#[derive(Debug, Clone)]
pub struct LexiconEntry {
    pub text: String,
    /// Syllable sequence, for example `bu ru`.
    pub code: String,
    /// Original preedit as written in the dictionary file.
    pub preedit: String,
    /// Number of input letters consumed by the full syllable sequence.
    pub consumed_keys: usize,
    /// Dictionary weight from the third tab-separated column.
    pub prior: f32,
}

impl LexiconEntry {
    /// Logarithmic dictionary weight used as the candidate base score.
    pub fn prior_score(&self) -> f32 {
        (self.prior.max(0.0) + 1.0).ln() * 0.15
    }
}

#[derive(Debug, Default)]
pub struct Lexicon {
    by_code: HashMap<String, Vec<LexiconEntry>>,
    syllables: HashSet<String>,
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
        lexicon.syllables = lexicon
            .by_code
            .keys()
            .flat_map(|code| code.split(' ').map(str::to_string))
            .collect();
        if lexicon.by_code.is_empty() {
            warn!(root = %root.display(), "no Rime dictionary entries were loaded");
        }
        lexicon
    }

    /// Build an in-memory lexicon for tests and small custom dictionaries.
    pub fn from_entries(entries: Vec<LexiconEntry>) -> Self {
        let mut lexicon = Self::default();
        for entry in entries {
            lexicon.insert(entry);
        }
        for entries in lexicon.by_code.values_mut() {
            entries.sort_by(|left, right| right.prior.total_cmp(&left.prior));
            entries.dedup_by(|right, left| right.text == left.text);
        }
        lexicon.syllables = lexicon
            .by_code
            .keys()
            .flat_map(|code| code.split(' ').map(str::to_string))
            .collect();
        lexicon
    }

    pub fn insert(&mut self, entry: LexiconEntry) {
        self.syllables
            .extend(entry.code.split(' ').map(str::to_string));
        let entries = self.by_code.entry(entry.code.clone()).or_default();
        if let Some(existing) = entries.iter_mut().find(|item| item.text == entry.text) {
            if entry.prior > existing.prior {
                existing.prior = entry.prior;
            }
        } else {
            entries.push(entry);
        }
    }

    /// Exact-match lookup for a segmented syllable sequence such as
    /// `["bu", "ru"]`.
    pub fn entries_for(&self, syllables: &[&str]) -> Vec<LexiconEntry> {
        let key = join_syllables(syllables);
        if key.is_empty() {
            return Vec::new();
        }
        self.by_code.get(&key).cloned().unwrap_or_default()
    }

    pub fn top_entries(&self, max_entries: usize) -> Vec<LexiconEntry> {
        let mut entries = self.by_code.values().flatten().cloned().collect::<Vec<_>>();
        entries.sort_by(|left, right| right.prior.total_cmp(&left.prior));
        entries.dedup_by(|right, left| right.text == left.text);
        entries.truncate(max_entries);
        entries
    }

    /// Valid pinyin syllables derived from the loaded dictionary codes.
    pub fn syllables(&self) -> &HashSet<String> {
        &self.syllables
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
            let before = self
                .by_code
                .get(&entry.code)
                .map_or(0, |entries| entries.len());
            let code = entry.code.clone();
            self.insert(entry);
            let after = self.by_code[&code].len();
            if after > before {
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
    let code = normalize_syllables(&preedit);
    if text.is_empty() || code.is_empty() || !is_chinese_text(text) || !is_syllable_code(&code) {
        return None;
    }
    let prior = fields
        .next()
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1.0);
    Some(LexiconEntry {
        text: text.to_string(),
        consumed_keys: code.chars().filter(|character| *character != ' ').count(),
        code,
        preedit,
        prior,
    })
}

/// Collapse whitespace runs into single spaces and lowercase the result so
/// `xi an`, `xi'an`, and `Xi AN` all become the same syllable key.
fn normalize_syllables(value: &str) -> String {
    value
        .replace(['\'', '-'], " ")
        .split_whitespace()
        .map(|syllable| syllable.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn join_syllables(syllables: &[&str]) -> String {
    syllables
        .iter()
        .filter(|syllable| !syllable.is_empty())
        .map(|syllable| syllable.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_chinese_text(text: &str) -> bool {
    text.chars().any(|character| {
        let code = character as u32;
        (0x3400..=0x4dbf).contains(&code)
            || (0x4e00..=0x9fff).contains(&code)
            || (0xf900..=0xfaff).contains(&code)
    })
}

fn is_syllable_code(code: &str) -> bool {
    code.split(' ').all(|syllable| {
        !syllable.is_empty()
            && syllable
                .chars()
                .all(|character| character.is_ascii_lowercase())
    })
}

fn dictionary_files(root: &Path) -> Vec<std::path::PathBuf> {
    let manifest = root.join("rime_ice.dict.yaml");
    if let Ok(contents) = fs::read_to_string(&manifest) {
        let imports = parse_imports(&contents);
        if !imports.is_empty() {
            let mut files = vec![manifest];
            for import in imports {
                if let Some(path) = resolve_import(root, &import) {
                    if !files.contains(&path) {
                        files.push(path);
                    }
                }
            }
            return files;
        }
    }

    let mut files = vec![manifest];
    if let Ok(entries) = fs::read_dir(root.join("cn_dicts")) {
        files.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml")),
        );
    }
    files[1..].sort();
    files
}

fn parse_imports(contents: &str) -> Vec<String> {
    let mut in_import_tables = false;
    let mut imports = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "import_tables:" {
            in_import_tables = true;
            continue;
        }
        if in_import_tables && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        if !in_import_tables {
            continue;
        }

        let Some(value) = trimmed.strip_prefix('-') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or_default().trim();
        let value = value.trim_matches(|character| character == '\'' || character == '"');
        if !value.is_empty() {
            imports.push(value.to_string());
        }
    }
    imports
}

fn resolve_import(root: &Path, import: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(import);
    let candidates = [
        root.join(path),
        root.join(format!("{import}.dict.yaml")),
        root.join(format!("{import}.yaml")),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_weighted_chinese_entry() {
        let entry = parse_entry("不如\tbu ru\t479325").unwrap();
        assert_eq!(entry.code, "bu ru");
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
    fn normalizes_apostrophes_and_whitespace_into_syllable_keys() {
        assert_eq!(normalize_syllables("xi'an"), "xi an");
        assert_eq!(normalize_syllables("Xi  AN"), "xi an");
        assert_eq!(join_syllables(&["XI", "an"]), "xi an");
    }

    #[test]
    fn syllable_keys_keep_segmentation_distinct() {
        let xi_an = parse_entry("西安\txi an\t100").unwrap();
        let xian = parse_entry("先\txian\t100").unwrap();
        assert_eq!(xi_an.code, "xi an");
        assert_eq!(xian.code, "xian");
        assert_ne!(xi_an.code, xian.code);
    }

    #[test]
    fn loads_bundled_dictionary_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/rime-ice");
        let lexicon = Lexicon::load(&root);
        let candidates = lexicon.entries_for(&["bu", "ru"]);

        assert!(candidates.iter().any(|entry| entry.text == "不如"));
        assert!(lexicon.by_code.len() > 100_000);
    }

    #[test]
    fn parses_only_enabled_import_tables() {
        let contents = r#"
import_tables:
  - cn_dicts/8105     # enabled
  # - cn_dicts/41448   # disabled
  - 'cn_dicts/base'

other:
  - should_not_be_loaded
"#;

        assert_eq!(
            parse_imports(contents),
            vec!["cn_dicts/8105", "cn_dicts/base"]
        );
    }
}

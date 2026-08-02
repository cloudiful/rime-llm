//! User word frequency with atomic file persistence. Frequencies stay on
//! disk and are never uploaded anywhere; the daemon writes them under
//! Application Support.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tracing::{debug, warn};

/// Extra prior added to a word's base score per logged selection.
const FREQ_BOOST_PER_COUNT: f32 = 0.08;

#[derive(Debug, Default)]
pub struct UserFreq {
    counts: HashMap<String, u32>,
    path: Option<PathBuf>,
}

impl UserFreq {
    pub fn load(path: &Path) -> Self {
        let mut counts = HashMap::new();
        match fs::read_to_string(path) {
            Ok(contents) => {
                for line in contents.lines() {
                    let Some((word, count)) = line.split_once('\t') else {
                        continue;
                    };
                    if word.is_empty() {
                        continue;
                    }
                    let Ok(count) = count.trim().parse::<u32>() else {
                        continue;
                    };
                    counts.insert(word.to_string(), count.max(1));
                }
                debug!(path = %path.display(), words = counts.len(), "loaded user frequency");
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "no user frequency file yet");
            }
            Err(error) => warn!(path = %path.display(), %error, "failed to read user frequency"),
        }
        Self {
            counts,
            path: Some(path.to_path_buf()),
        }
    }

    /// Record one selection of `word` and persist atomically.
    pub fn record(&mut self, word: &str) {
        let count = self.counts.entry(word.to_string()).or_default();
        *count = count.saturating_add(1);
        if let Some(path) = &self.path {
            if let Err(error) = save_atomic(path, &self.counts) {
                warn!(path = %path.display(), %error, "failed to persist user frequency");
            }
        }
    }

    pub fn count(&self, word: &str) -> u32 {
        self.counts.get(word).copied().unwrap_or(0)
    }

    /// Frequency contribution to a candidate's base score.
    pub fn boost(&self, word: &str) -> f32 {
        let count = self.count(word);
        if count == 0 {
            0.0
        } else {
            (count as f32).ln() * FREQ_BOOST_PER_COUNT + FREQ_BOOST_PER_COUNT
        }
    }
}

fn save_atomic(path: &Path, counts: &HashMap<String, u32>) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let temp = path.with_extension("tmp");
    let mut file = fs::File::create(&temp)?;
    for (word, count) in counts {
        writeln!(file, "{word}\t{count}")?;
    }
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_increments_and_boosts() {
        let mut freq = UserFreq::default();
        assert_eq!(freq.count("苹果"), 0);
        assert_eq!(freq.boost("苹果"), 0.0);
        freq.record("苹果");
        freq.record("苹果");
        assert_eq!(freq.count("苹果"), 2);
        assert!(freq.boost("苹果") > freq.boost("香蕉"));
    }

    #[test]
    fn persistence_round_trip_is_atomic() {
        let directory =
            std::env::temp_dir().join(format!("rime-freq-test-{}-roundtrip", std::process::id()));
        let path = directory.join("user_freq.txt");
        let _ = fs::remove_dir_all(&directory);

        let mut freq = UserFreq::load(&path);
        freq.record("苹果");
        freq.record("香蕉");
        freq.record("苹果");
        assert!(
            !path.with_extension("tmp").exists(),
            "temp file must be renamed"
        );

        let reloaded = UserFreq::load(&path);
        assert_eq!(reloaded.count("苹果"), 2);
        assert_eq!(reloaded.count("香蕉"), 1);

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn load_skips_invalid_lines() {
        let directory =
            std::env::temp_dir().join(format!("rime-freq-test-{}-invalid", std::process::id()));
        let path = directory.join("user_freq.txt");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "苹果\t2\n坏行\n香蕉\tabc\n").unwrap();

        let freq = UserFreq::load(&path);
        assert_eq!(freq.count("苹果"), 2);
        assert_eq!(freq.count("香蕉"), 0);

        let _ = fs::remove_dir_all(&directory);
    }
}

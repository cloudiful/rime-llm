//! Pinyin syllable table and input segmentation into syllable edges.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct SyllableEdge {
    /// Letter index where the syllable starts.
    pub start: usize,
    /// Letter index after the last letter of the syllable.
    pub end: usize,
    pub syllable: String,
}

#[derive(Debug)]
pub struct SyllableTable {
    by_first: HashMap<char, Vec<String>>,
}

impl SyllableTable {
    pub fn from_set(syllables: &HashSet<String>) -> Self {
        let mut by_first: HashMap<char, Vec<String>> = HashMap::new();
        for syllable in syllables {
            if let Some(first) = syllable.chars().next() {
                by_first.entry(first).or_default().push(syllable.clone());
            }
        }
        for entries in by_first.values_mut() {
            entries.sort();
            entries.dedup();
        }
        Self { by_first }
    }

    /// All valid syllable edges reachable from position 0. Apostrophes are
    /// forced syllable boundaries that consume no letters.
    pub fn edges(&self, input: &str) -> Vec<SyllableEdge> {
        let bytes = input.as_bytes();
        let mut reachable = vec![false; bytes.len() + 1];
        reachable[0] = true;
        let mut edges = Vec::new();

        for start in 0..bytes.len() {
            if !reachable[start] {
                continue;
            }
            if bytes[start] == b'\'' {
                reachable[start + 1] = true;
                continue;
            }
            let Some(first) = input[start..].chars().next() else {
                continue;
            };
            let Some(candidates) = self.by_first.get(&first) else {
                continue;
            };
            for syllable in candidates {
                let end = start + syllable.len();
                if end <= bytes.len() && input[start..end] == *syllable {
                    reachable[end] = true;
                    edges.push(SyllableEdge {
                        start,
                        end,
                        syllable: syllable.clone(),
                    });
                }
            }
        }
        edges
    }
}

/// Display form of the composition: the longest valid syllables from the
/// start joined with spaces, with the unsegmentable tail kept verbatim.
pub fn spaced_preedit(input: &str, table: &SyllableTable) -> String {
    spaced_preedit_with_cursor(input, input.len(), table).0
}

/// Returns the display preedit and the cursor offset within that display.
///
/// `InputState::cursor` indexes the raw ASCII pinyin buffer, while the
/// display form inserts spaces and removes apostrophe boundaries. Keeping the
/// mapping here ensures every frontend uses the same offset convention.
pub fn spaced_preedit_with_cursor(
    input: &str,
    cursor: usize,
    table: &SyllableTable,
) -> (String, usize) {
    let segments = preedit_segments(input, table);
    if segments.is_empty() {
        return (String::new(), 0);
    }

    let mut preedit = String::new();
    let mut display_ranges = Vec::with_capacity(segments.len());
    for (index, (start, end, text)) in segments.iter().enumerate() {
        if index > 0 {
            preedit.push(' ');
        }
        let display_start = preedit.len();
        preedit.push_str(text);
        let display_end = preedit.len();
        display_ranges.push((*start, *end, display_start, display_end));
    }

    let cursor = cursor.min(input.len());
    let display_cursor = display_ranges
        .iter()
        .enumerate()
        .find_map(|(index, (start, end, display_start, display_end))| {
            if cursor < *start {
                return Some(*display_start);
            }
            if cursor < *end {
                return Some(*display_start + cursor - *start);
            }
            if cursor == *end {
                return Some(if index + 1 < display_ranges.len() {
                    display_end + 1
                } else {
                    *display_end
                });
            }
            None
        })
        .unwrap_or(preedit.len());

    (preedit, display_cursor)
}

fn preedit_segments(input: &str, table: &SyllableTable) -> Vec<(usize, usize, String)> {
    let bytes = input.as_bytes();
    let mut segments = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] == b'\'' {
            position += 1;
            continue;
        }
        let Some(first) = input[position..].chars().next() else {
            break;
        };
        let mut matched: Option<&str> = None;
        if let Some(candidates) = table.by_first.get(&first) {
            for syllable in candidates {
                let end = position + syllable.len();
                if end <= bytes.len() && input[position..end] == *syllable {
                    if matched.map_or(true, |current| syllable.len() > current.len()) {
                        matched = Some(syllable);
                    }
                }
            }
        }
        if let Some(syllable) = matched {
            let end = position + syllable.len();
            segments.push((position, end, syllable.to_string()));
            position = end;
        } else {
            segments.push((position, bytes.len(), input[position..].to_string()));
            break;
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(syllables: &[&str]) -> SyllableTable {
        SyllableTable::from_set(&syllables.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn finds_all_segmentations_of_ambiguous_input() {
        let table = table(&["xian", "xi", "an", "x", "i", "a", "n", "xia"]);
        let edges = table.edges("xian");
        let spans = edges
            .iter()
            .map(|edge| (edge.start, edge.end, edge.syllable.as_str()))
            .collect::<Vec<_>>();
        assert!(spans.contains(&(0, 2, "xi")));
        assert!(spans.contains(&(0, 3, "xia")));
        assert!(spans.contains(&(0, 4, "xian")));
        assert!(spans.contains(&(2, 4, "an")));
    }

    #[test]
    fn apostrophe_is_a_forced_boundary() {
        let table = table(&["xi", "an", "xian"]);
        let edges = table.edges("xi'an");
        let spans = edges
            .iter()
            .map(|edge| (edge.start, edge.end))
            .collect::<Vec<_>>();
        assert!(spans.contains(&(0, 2)));
        assert!(spans.contains(&(3, 5)));
        assert!(
            !spans.contains(&(0, 5)),
            "xian must not cross the apostrophe"
        );
    }

    #[test]
    fn invalid_letters_produce_no_edges() {
        let table = table(&["bu", "ru"]);
        assert!(table.edges("b").is_empty());
        assert!(table.edges("bur").iter().any(|edge| edge.syllable == "bu"));
    }

    #[test]
    fn spaced_preedit_joins_syllables_and_keeps_tail() {
        let table = table(&["bu", "ru", "xi", "an", "xian"]);
        assert_eq!(spaced_preedit("buru", &table), "bu ru");
        assert_eq!(spaced_preedit("xi'an", &table), "xi an");
        assert_eq!(spaced_preedit("bunr", &table), "bu nr");
    }

    #[test]
    fn preedit_cursor_accounts_for_inserted_spaces() {
        let table = table(&["bu", "ru"]);
        assert_eq!(
            spaced_preedit_with_cursor("buru", 2, &table),
            ("bu ru".into(), 3)
        );
        assert_eq!(
            spaced_preedit_with_cursor("buru", 3, &table),
            ("bu ru".into(), 4)
        );
        assert_eq!(
            spaced_preedit_with_cursor("buru", 4, &table),
            ("bu ru".into(), 5)
        );
    }

    #[test]
    fn preedit_cursor_handles_apostrophe_boundaries() {
        let table = table(&["xi", "an"]);
        assert_eq!(
            spaced_preedit_with_cursor("xi'an", 2, &table),
            ("xi an".into(), 3)
        );
        assert_eq!(
            spaced_preedit_with_cursor("xi'an", 3, &table),
            ("xi an".into(), 3)
        );
    }
}

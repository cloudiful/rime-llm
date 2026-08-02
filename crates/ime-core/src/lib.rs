//! Pure input method core: pinyin syllable segmentation, dictionary
//! candidate lattice with k-best path search, and the composition state
//! machine. No I/O and no model access; the daemon owns those.

pub mod lattice;
pub mod state;
pub mod syllable;

pub use lattice::{Candidate, CandidateEngine, DEFAULT_MAX_CANDIDATES};
pub use state::{Effect, InputEvent, InputState, StateMachine, PAGE_SIZE};

/// Keep lowercase letters and apostrophes (forced syllable boundaries).
pub fn normalize_input(input: &str) -> String {
    input
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .filter(|character| character.is_ascii_lowercase() || *character == '\'')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_input_keeps_letters_and_boundaries() {
        assert_eq!(normalize_input("Ni Hao! 不如 bu-ru"), "nihaoburu");
        assert_eq!(normalize_input("XI'AN"), "xi'an");
    }
}

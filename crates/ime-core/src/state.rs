//! Composition state machine: maps input events to state transitions and
//! commit/clear effects. Candidate recomputation stays with the daemon.

use model_protocol::PredictionCandidate;

use crate::lattice::Candidate;

pub const PAGE_SIZE: usize = 9;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputState {
    /// Raw lowercased input letters (plus apostrophe boundaries).
    pub input: String,
    /// Insertion/backspace position inside `input`.
    pub cursor: usize,
    pub candidates: Vec<Candidate>,
    pub selected_index: usize,
    pub page: usize,
    pub predictions: Vec<PredictionCandidate>,
    pub model_pending: bool,
    /// Bumped on every input-changing key; guards stale model responses.
    pub revision: u64,
}

impl InputState {
    pub fn selection(&self) -> Option<&Candidate> {
        self.candidates.get(self.selected_index)
    }

    pub fn page_count(&self) -> usize {
        self.candidates.len().div_ceil(PAGE_SIZE).max(1)
    }

    /// Restores the selection and page invariants after candidates are
    /// reordered or reduced by an asynchronous model response.
    pub fn normalize_selection(&mut self) {
        if self.candidates.is_empty() {
            self.selected_index = 0;
            self.page = 0;
            return;
        }
        self.selected_index = self.selected_index.min(self.candidates.len() - 1);
        self.page = self.selected_index / PAGE_SIZE;
    }

    fn reset_composition(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.candidates.clear();
        self.selected_index = 0;
        self.page = 0;
        self.predictions.clear();
        self.model_pending = false;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Letter(char),
    Backspace,
    Delete,
    Left,
    Right,
    PageUp,
    PageDown,
    Space,
    Enter,
    Escape,
    Digit(u8),
    SelectCandidate(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    None,
    /// Commit `text` and keep composing the remaining input.
    Commit {
        text: String,
    },
    /// Commit `text` and end the composition.
    CommitAndClear {
        text: String,
    },
    Clear,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub effect: Effect,
    /// True when the composition input changed and candidates are stale.
    pub candidates_dirty: bool,
}

pub struct StateMachine;

impl StateMachine {
    pub fn apply(event: &InputEvent, state: &mut InputState) -> Outcome {
        match event {
            InputEvent::Letter(character) if is_editable(*character) => {
                state.input.insert(state.cursor, *character);
                state.cursor = state.cursor.saturating_add(1);
                state.revision = state.revision.wrapping_add(1);
                dirty()
            }
            InputEvent::Letter(_) => Outcome {
                effect: Effect::None,
                candidates_dirty: false,
            },
            InputEvent::Backspace => {
                if state.cursor == 0 {
                    return Outcome {
                        effect: Effect::None,
                        candidates_dirty: false,
                    };
                }
                state.input.remove(state.cursor - 1);
                state.cursor -= 1;
                state.revision = state.revision.wrapping_add(1);
                dirty()
            }
            InputEvent::Delete => {
                if state.cursor >= state.input.len() {
                    return Outcome {
                        effect: Effect::None,
                        candidates_dirty: false,
                    };
                }
                state.input.remove(state.cursor);
                state.revision = state.revision.wrapping_add(1);
                dirty()
            }
            InputEvent::Left => {
                state.cursor = state.cursor.saturating_sub(1);
                Outcome {
                    effect: Effect::None,
                    candidates_dirty: false,
                }
            }
            InputEvent::Right => {
                state.cursor = state.cursor.saturating_add(1).min(state.input.len());
                Outcome {
                    effect: Effect::None,
                    candidates_dirty: false,
                }
            }
            InputEvent::PageUp => {
                state.page = state.page.saturating_sub(1);
                state.selected_index = state.page * PAGE_SIZE;
                state.normalize_selection();
                Outcome {
                    effect: Effect::None,
                    candidates_dirty: false,
                }
            }
            InputEvent::PageDown => {
                if (state.page + 1) * PAGE_SIZE < state.candidates.len() {
                    state.page += 1;
                    state.selected_index = state.page * PAGE_SIZE;
                }
                state.normalize_selection();
                Outcome {
                    effect: Effect::None,
                    candidates_dirty: false,
                }
            }
            InputEvent::Space => {
                if state.candidates.is_empty() {
                    state.revision = state.revision.wrapping_add(1);
                    let text = state.input.clone();
                    state.reset_composition();
                    return Outcome {
                        effect: Effect::CommitAndClear { text },
                        candidates_dirty: true,
                    };
                }
                let index = state.page * PAGE_SIZE;
                Outcome {
                    effect: commit_candidate(state, index),
                    candidates_dirty: true,
                }
            }
            InputEvent::Enter => {
                if state.candidates.is_empty() {
                    state.revision = state.revision.wrapping_add(1);
                    let text = state.input.clone();
                    state.reset_composition();
                    return Outcome {
                        effect: Effect::CommitAndClear { text },
                        candidates_dirty: true,
                    };
                }
                let index = state.selected_index;
                Outcome {
                    effect: commit_candidate(state, index),
                    candidates_dirty: true,
                }
            }
            InputEvent::Escape => {
                state.revision = state.revision.wrapping_add(1);
                state.reset_composition();
                Outcome {
                    effect: Effect::Clear,
                    candidates_dirty: true,
                }
            }
            InputEvent::Digit(digit) => {
                if *digit == 0 || *digit > PAGE_SIZE as u8 {
                    return Outcome {
                        effect: Effect::None,
                        candidates_dirty: false,
                    };
                }
                let index = state.page * PAGE_SIZE + (*digit as usize - 1);
                if index >= state.candidates.len() {
                    return Outcome {
                        effect: Effect::None,
                        candidates_dirty: false,
                    };
                }
                Outcome {
                    effect: commit_candidate(state, index),
                    candidates_dirty: true,
                }
            }
            InputEvent::SelectCandidate(index) => {
                if *index >= state.candidates.len() {
                    return Outcome {
                        effect: Effect::None,
                        candidates_dirty: false,
                    };
                }
                Outcome {
                    effect: commit_candidate(state, *index),
                    candidates_dirty: true,
                }
            }
        }
    }
}

fn commit_candidate(state: &mut InputState, index: usize) -> Effect {
    let Some(candidate) = state.candidates.get(index).cloned() else {
        return Effect::None;
    };
    let consumed = candidate.consumedkeys.min(state.input.len());
    let remaining = state.input[consumed..].to_string();
    state.revision = state.revision.wrapping_add(1);
    state.input = remaining.clone();
    state.cursor = state.cursor.saturating_sub(consumed).min(state.input.len());
    state.candidates.clear();
    state.selected_index = 0;
    state.page = 0;
    state.predictions.clear();
    state.model_pending = false;

    if remaining.is_empty() {
        Effect::CommitAndClear {
            text: candidate.text,
        }
    } else {
        Effect::Commit {
            text: candidate.text,
        }
    }
}

fn dirty() -> Outcome {
    Outcome {
        effect: Effect::None,
        candidates_dirty: true,
    }
}

fn is_editable(character: char) -> bool {
    character.is_ascii_lowercase() || character == '\''
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(text: &str, consumedkeys: usize) -> Candidate {
        Candidate {
            id: format!("d:{text}"),
            text: text.to_string(),
            preedit: "bu ru".to_string(),
            consumedkeys,
            base_score: 1.0,
            kind: "dictionary".to_string(),
        }
    }

    fn state_with_input(input: &str) -> InputState {
        InputState {
            input: input.to_string(),
            cursor: input.len(),
            ..InputState::default()
        }
    }

    #[test]
    fn letters_insert_at_cursor_and_bump_revision() {
        let mut state = InputState::default();
        for character in ['b', 'u'] {
            let outcome = StateMachine::apply(&InputEvent::Letter(character), &mut state);
            assert!(outcome.candidates_dirty);
            assert_eq!(outcome.effect, Effect::None);
        }
        assert_eq!(state.input, "bu");
        assert_eq!(state.revision, 2);

        state.cursor = 1;
        StateMachine::apply(&InputEvent::Letter('x'), &mut state);
        assert_eq!(state.input, "bxu");
    }

    #[test]
    fn backspace_and_delete_respect_cursor() {
        let mut state = state_with_input("buru");
        state.cursor = 3;
        StateMachine::apply(&InputEvent::Backspace, &mut state);
        assert_eq!(state.input, "buu");
        assert_eq!(state.cursor, 2);

        StateMachine::apply(&InputEvent::Delete, &mut state);
        assert_eq!(state.input, "bu");
        StateMachine::apply(&InputEvent::Right, &mut state);
        StateMachine::apply(&InputEvent::Delete, &mut state);
        assert_eq!(state.input, "bu");
    }

    #[test]
    fn space_commits_first_candidate_and_keeps_remainder() {
        let mut state = state_with_input("burushang");
        state.candidates = vec![candidate("不如", 4), candidate("不入", 4)];
        let outcome = StateMachine::apply(&InputEvent::Space, &mut state);
        assert_eq!(
            outcome.effect,
            Effect::Commit {
                text: "不如".to_string()
            }
        );
        assert_eq!(state.input, "shang");
        assert!(state.candidates.is_empty());
    }

    #[test]
    fn full_consumption_commits_and_clears() {
        let mut state = state_with_input("buru");
        state.candidates = vec![candidate("不如", 4)];
        let outcome = StateMachine::apply(&InputEvent::Digit(1), &mut state);
        assert_eq!(
            outcome.effect,
            Effect::CommitAndClear {
                text: "不如".to_string()
            }
        );
        assert!(state.input.is_empty());
    }

    #[test]
    fn enter_commits_selected_candidate_or_raw_input() {
        let mut state = state_with_input("buru");
        state.candidates = vec![candidate("不如", 4), candidate("不入", 4)];
        state.selected_index = 1;
        let outcome = StateMachine::apply(&InputEvent::Enter, &mut state);
        assert_eq!(
            outcome.effect,
            Effect::CommitAndClear {
                text: "不入".to_string()
            }
        );

        let mut raw = state_with_input("buru");
        let outcome = StateMachine::apply(&InputEvent::Enter, &mut raw);
        assert_eq!(
            outcome.effect,
            Effect::CommitAndClear {
                text: "buru".to_string()
            }
        );
    }

    #[test]
    fn space_with_no_candidates_commits_raw_input() {
        let mut state = state_with_input("buru");
        let outcome = StateMachine::apply(&InputEvent::Space, &mut state);
        assert_eq!(
            outcome.effect,
            Effect::CommitAndClear {
                text: "buru".to_string()
            }
        );
        assert!(state.input.is_empty());
        assert!(state.candidates.is_empty());
    }

    #[test]
    fn escape_clears_composition() {
        let mut state = state_with_input("buru");
        state.candidates = vec![candidate("不如", 4)];
        let outcome = StateMachine::apply(&InputEvent::Escape, &mut state);
        assert_eq!(outcome.effect, Effect::Clear);
        assert!(state.input.is_empty());
        assert!(state.candidates.is_empty());
    }

    #[test]
    fn digits_select_by_page_offset() {
        let mut state = state_with_input("shi");
        state.candidates = (0..15)
            .map(|index| candidate(&format!("字{index}"), 3))
            .collect();
        state.page = 1;
        let outcome = StateMachine::apply(&InputEvent::Digit(2), &mut state);
        assert_eq!(
            outcome.effect,
            Effect::CommitAndClear {
                text: "字10".to_string()
            }
        );
    }

    #[test]
    fn out_of_range_digits_are_ignored() {
        let mut state = state_with_input("bu");
        state.candidates = vec![candidate("不", 2)];
        let outcome = StateMachine::apply(&InputEvent::Digit(9), &mut state);
        assert_eq!(outcome.effect, Effect::None);
        assert!(!outcome.candidates_dirty);
        assert_eq!(state.input, "bu");
    }

    #[test]
    fn paging_is_bounded_and_moves_selection() {
        let mut state = state_with_input("shi");
        state.candidates = (0..15)
            .map(|index| candidate(&format!("字{index}"), 3))
            .collect();
        StateMachine::apply(&InputEvent::PageDown, &mut state);
        assert_eq!(state.page, 1);
        assert_eq!(state.selected_index, PAGE_SIZE);
        StateMachine::apply(&InputEvent::PageDown, &mut state);
        assert_eq!(state.page, 1, "last page is not overrun");
        StateMachine::apply(&InputEvent::PageUp, &mut state);
        assert_eq!(state.page, 0);
    }

    #[test]
    fn selection_normalizes_after_candidates_shrink() {
        let mut state = state_with_input("shi");
        state.candidates = (0..15)
            .map(|index| candidate(&format!("字{index}"), 3))
            .collect();
        state.page = 1;
        state.selected_index = 12;

        state.candidates.truncate(2);
        state.normalize_selection();

        assert_eq!(state.selected_index, 1);
        assert_eq!(state.page, 0);
    }

    #[test]
    fn cursor_moves_do_not_bump_revision() {
        let mut state = state_with_input("buru");
        let revision = state.revision;
        StateMachine::apply(&InputEvent::Left, &mut state);
        StateMachine::apply(&InputEvent::Right, &mut state);
        assert_eq!(state.revision, revision);
    }
}

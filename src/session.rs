use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    max_chars: usize,
}

#[derive(Default)]
struct Session {
    commits: VecDeque<String>,
    revision: u64,
}

impl SessionStore {
    pub fn new(max_chars: usize) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            max_chars,
        }
    }

    pub async fn context(&self, session_id: &str) -> Vec<String> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|session| session.commits.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn revision(&self, session_id: &str) -> u64 {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|session| session.revision)
            .unwrap_or_default()
    }

    pub async fn commit(&self, session_id: &str, text: &str) -> u64 {
        if session_id.is_empty() || text.is_empty() {
            return self.revision(session_id).await;
        }
        let mut sessions = self.sessions.lock().await;
        let session = sessions.entry(session_id.to_string()).or_default();
        session.commits.push_back(text.to_string());
        session.revision = session.revision.wrapping_add(1);
        trim_context(session, self.max_chars);
        session.revision
    }

    pub async fn reset(&self, session_id: &str) -> u64 {
        if session_id.is_empty() {
            return 0;
        }
        let mut sessions = self.sessions.lock().await;
        let session = sessions.entry(session_id.to_string()).or_default();
        session.commits.clear();
        session.revision = session.revision.wrapping_add(1);
        session.revision
    }
}

fn trim_context(session: &mut Session, max_chars: usize) {
    loop {
        let total = session
            .commits
            .iter()
            .map(|item| item.chars().count())
            .sum::<usize>();
        if session.commits.len() <= 8 && total <= max_chars {
            break;
        }
        if session.commits.len() > 1 {
            session.commits.pop_front();
            continue;
        }
        if let Some(last) = session.commits.back_mut() {
            *last = last
                .chars()
                .rev()
                .take(max_chars)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
        }
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sessions_are_isolated_and_trimmed() {
        let store = SessionStore::new(2);
        store.commit("a", "甲").await;
        store.commit("a", "乙").await;
        store.commit("b", "丙").await;
        assert_eq!(store.context("a").await, vec!["甲", "乙"]);
        assert_eq!(store.context("b").await, vec!["丙"]);

        store.commit("a", "很长").await;
        assert_eq!(store.context("a").await.join(""), "很长");
    }

    #[tokio::test]
    async fn reset_does_not_touch_other_sessions() {
        let store = SessionStore::new(20);
        store.commit("a", "甲").await;
        store.commit("b", "乙").await;
        assert_eq!(store.reset("a").await, 2);
        assert!(store.context("a").await.is_empty());
        assert_eq!(store.context("b").await, vec!["乙"]);
    }

    #[tokio::test]
    async fn revision_changes_on_commit_and_reset() {
        let store = SessionStore::new(20);
        assert_eq!(store.revision("a").await, 0);
        assert_eq!(store.commit("a", "甲").await, 1);
        assert_eq!(store.reset("a").await, 2);
        assert_eq!(store.revision("a").await, 2);
    }
}

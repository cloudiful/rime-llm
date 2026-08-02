//! Shared test doubles: a scriptable model client and a small lexicon.

use std::sync::Arc;

use model_protocol::{CandidatesResponse, ModelCandidate, PredictionCandidate, PredictionResponse};
use pinyin_dict::{Lexicon, LexiconEntry, UserFreq};
use tokio::sync::Mutex;

use crate::{
    config::Config,
    model_client::{MockDriver, MockModelApi, ModelClient},
    session::SessionStore,
};

pub fn entry(text: &str, code: &str, prior: f32) -> LexiconEntry {
    LexiconEntry {
        text: text.to_string(),
        code: code.to_string(),
        preedit: code.to_string(),
        consumed_keys: code.chars().filter(|character| *character != ' ').count(),
        prior,
    }
}

pub fn test_store() -> (Arc<SessionStore>, MockDriver) {
    let lexicon = Arc::new(Lexicon::from_entries(vec![
        entry("不如", "bu ru", 479_325.0),
        entry("不入", "bu ru", 20_830.0),
        entry("不", "bu", 100.0),
    ]));
    let (mock, driver) = MockModelApi::new();
    let store = Arc::new(SessionStore::new(
        lexicon,
        Arc::new(Mutex::new(UserFreq::default())),
        Config {
            bind_addr: "127.0.0.1:0".to_string(),
            service_url: "http://mock".to_string(),
            dictionary_root: ".".into(),
            user_freq_path: std::env::temp_dir()
                .join(format!("ime-daemon-test-{}.freq", std::process::id()))
                .to_string_lossy()
                .into_owned()
                .into(),
            max_candidates: 16,
            model_timeout_ms: 1000,
        },
        Arc::new(ModelClient::Mock(mock)),
    ));
    (store, driver)
}

pub fn model_response(ids: &[&str]) -> CandidatesResponse {
    CandidatesResponse {
        status: "ready".to_string(),
        candidates: ids
            .iter()
            .enumerate()
            .map(|(index, id)| ModelCandidate {
                id: id.to_string(),
                text: id.strip_prefix("d:").unwrap_or(id).to_string(),
                preedit: String::new(),
                consumedkeys: 4,
                score: -0.1 - index as f32,
                kind: "llm_phrase".to_string(),
            })
            .collect(),
        source: "model".to_string(),
        elapsed_ms: 1,
    }
}

pub fn prediction_response(text: &str) -> PredictionResponse {
    PredictionResponse {
        status: "ready".to_string(),
        revision: 1,
        candidates: vec![PredictionCandidate {
            id: "g0".to_string(),
            text: text.to_string(),
            score: 1.0,
            kind: "llm_prediction".to_string(),
        }],
        source: "generation".to_string(),
        elapsed_ms: 1,
    }
}

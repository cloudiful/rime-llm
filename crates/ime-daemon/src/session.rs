//! Per-client sessions: composition state, model request channel, and the
//! wire snapshot broadcast to WebSocket subscribers.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use ime_core::{Candidate, InputState};
use model_protocol::PredictionCandidate;
use pinyin_dict::{Lexicon, UserFreq};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch, Mutex};

use crate::{config::Config, model_client::ModelClient};

pub struct SessionStore {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub(crate) lexicon: Arc<Lexicon>,
    pub(crate) user_freq: Arc<Mutex<UserFreq>>,
    pub(crate) config: Arc<Config>,
    pub(crate) model: Arc<ModelClient>,
}

impl SessionStore {
    pub fn new(
        lexicon: Arc<Lexicon>,
        user_freq: Arc<Mutex<UserFreq>>,
        config: Config,
        model: Arc<ModelClient>,
    ) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            lexicon,
            user_freq,
            config: Arc::new(config),
            model,
        }
    }

    pub async fn create(&self) -> Arc<Session> {
        let session = Arc::new(Session::new(uuid::Uuid::new_v4().to_string()));
        self.sessions
            .lock()
            .await
            .insert(session.id.clone(), session.clone());
        session
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> bool {
        if let Some(session) = self.sessions.lock().await.remove(id) {
            let _ = session.shutdown.send(true);
            true
        } else {
            false
        }
    }

    /// Recomputed dictionary candidates after an input change. Lock order is
    /// always session state first, then user frequency.
    pub(crate) async fn refresh_candidates(&self, state: &mut InputState) {
        let freq = self.user_freq.lock().await;
        let engine = ime_core::CandidateEngine::new(&self.lexicon, &freq);
        state.candidates = engine.candidates_limited(&state.input, self.config.max_candidates);
        state.selected_index = 0;
        state.page = 0;
    }
}

pub struct Session {
    pub id: String,
    pub(crate) state: Mutex<InputState>,
    pub(crate) tx: broadcast::Sender<String>,
    pub(crate) seq: AtomicU64,
    pub(crate) service_revision: AtomicU64,
    pub(crate) model_req: watch::Sender<u64>,
    pub(crate) model_req_rx: watch::Receiver<u64>,
    pub(crate) shutdown: watch::Sender<bool>,
    pub(crate) shutdown_rx: watch::Receiver<bool>,
}

impl Session {
    fn new(id: String) -> Self {
        let (model_req, model_req_rx) = watch::channel(0);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (tx, _) = broadcast::channel(64);
        Self {
            id,
            state: Mutex::new(InputState::default()),
            tx,
            seq: AtomicU64::new(0),
            service_revision: AtomicU64::new(0),
            model_req,
            model_req_rx,
            shutdown,
            shutdown_rx,
        }
    }

    pub async fn snapshot(&self, store: &SessionStore, state: &InputState) -> StateSnapshot {
        let table = ime_core::syllable::SyllableTable::from_set(store.lexicon.syllables());
        let (preedit, preedit_cursor) =
            ime_core::syllable::spaced_preedit_with_cursor(&state.input, state.cursor, &table);
        StateSnapshot {
            composition: Composition {
                input: state.input.clone(),
                cursor: state.cursor,
                preedit_cursor,
            },
            preedit,
            candidates: state.candidates.iter().map(CandidateWire::from).collect(),
            selected_index: state.selected_index,
            page: state.page,
            page_size: ime_core::PAGE_SIZE,
            predictions: state.predictions.clone(),
            model_pending: state.model_pending,
            revision: state.revision,
            event_seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }

    pub async fn broadcast(&self, store: &SessionStore, state: &InputState) {
        let snapshot = self.snapshot(store, state).await;
        let Ok(payload) = serde_json::to_string(&snapshot) else {
            return;
        };
        let _ = self.tx.send(payload);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct StateSnapshot {
    pub composition: Composition,
    pub preedit: String,
    pub candidates: Vec<CandidateWire>,
    pub selected_index: usize,
    pub page: usize,
    pub page_size: usize,
    pub predictions: Vec<PredictionCandidate>,
    pub model_pending: bool,
    pub revision: u64,
    pub event_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Composition {
    pub input: String,
    pub cursor: usize,
    pub preedit_cursor: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct CandidateWire {
    pub id: String,
    pub text: String,
    pub preedit: String,
    pub consumedkeys: usize,
    pub base_score: f32,
    pub kind: String,
}

impl From<&Candidate> for CandidateWire {
    fn from(candidate: &Candidate) -> Self {
        Self {
            id: candidate.id.clone(),
            text: candidate.text.clone(),
            preedit: candidate.preedit.clone(),
            consumedkeys: candidate.consumedkeys,
            base_score: candidate.base_score,
            kind: candidate.kind.clone(),
        }
    }
}

//! Orchestration: key events update the state machine and dictionary
//! candidates synchronously; a per-session worker reranks with the model
//! asynchronously, coalescing keys and dropping stale revisions.

use std::sync::Arc;

use anyhow::{Context, Result};
use ime_core::{Effect, InputEvent, InputState, StateMachine};
use model_protocol::{
    CandidatePath, CandidatesRequest, CommitRequest, ModelCandidate, PredictionMode,
    PredictionRequest,
};
use tracing::{debug, warn};

use crate::session::{Session, SessionStore, StateSnapshot};

pub async fn handle_key(
    store: &SessionStore,
    session: &Session,
    event: InputEvent,
) -> Result<(StateSnapshot, Effect)> {
    let mut state = session.state.lock().await;
    let outcome = StateMachine::apply(&event, &mut state);
    if outcome.candidates_dirty {
        store.refresh_candidates(&mut state).await;
        if state.input.is_empty() {
            state.model_pending = false;
            session.broadcast(store, &state).await;
        } else {
            state.predictions.clear();
            state.model_pending = !state.candidates.is_empty();
            if state.model_pending {
                let _ = session.model_req.send(state.revision);
            }
        }
    }
    let snapshot = session.snapshot(store, &state).await;
    Ok((snapshot, outcome.effect))
}

/// Single worker per session: latest-wins coalescing of candidate reranks.
/// A model response only merges when the composition revision is unchanged.
pub async fn model_worker(store: Arc<SessionStore>, session: Arc<Session>) {
    let mut revision_rx = session.model_req_rx.clone();
    let mut shutdown_rx = session.shutdown_rx.clone();
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => return,
            result = revision_rx.changed() => {
                let Ok(()) = result else { return };
            }
        }
        let revision = *revision_rx.borrow();
        if revision == 0 {
            continue;
        }
        let (input, paths) = {
            let state = session.state.lock().await;
            if state.revision != revision {
                continue;
            }
            let paths = state
                .candidates
                .iter()
                .map(|candidate| CandidatePath {
                    id: candidate.id.clone(),
                    text: candidate.text.clone(),
                    preedit: candidate.preedit.clone(),
                    consumedkeys: candidate.consumedkeys,
                    base_score: candidate.base_score,
                })
                .collect::<Vec<_>>();
            (state.input.clone(), paths)
        };
        if input.is_empty() || paths.is_empty() {
            continue;
        }

        let request = CandidatesRequest {
            session_id: session.id.clone(),
            input: input.clone(),
            max_candidates: Some(store.config.max_candidates),
            paths,
        };
        debug!(revision, input, "requesting model candidate rerank");
        let result = tokio::select! {
            _ = shutdown_rx.changed() => return,
            result = store.model.candidates(&request) => result,
        };
        match result {
            Ok(response) if response.status == "ready" && !response.candidates.is_empty() => {
                let mut state = session.state.lock().await;
                if state.revision != revision {
                    continue;
                }
                merge_candidates(&mut state, &response.candidates);
                state.model_pending = false;
                session.broadcast(&store, &state).await;
            }
            Ok(response) => {
                debug!(revision, status = %response.status, "model rerank fell back");
                clear_candidate_pending(&store, &session, revision).await;
            }
            Err(error) => {
                warn!(revision, %error, "model rerank failed; keeping dictionary candidates");
                clear_candidate_pending(&store, &session, revision).await;
            }
        }
    }
}

pub async fn handle_commit_ack(
    store: Arc<SessionStore>,
    session: Arc<Session>,
    text: &str,
) -> Result<u64> {
    if text.is_empty() {
        return Ok(session
            .service_revision
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    store.user_freq.lock().await.record(text);
    let request = CommitRequest {
        session_id: session.id.clone(),
        text: text.to_string(),
    };
    let service_revision = store
        .model
        .commit(&request)
        .await
        .context("model commit failed")?;
    session
        .service_revision
        .store(service_revision, std::sync::atomic::Ordering::SeqCst);

    let empty = {
        let state = session.state.lock().await;
        state.input.is_empty()
    };
    if empty {
        let mut state = session.state.lock().await;
        state.model_pending = true;
        session.broadcast(store.as_ref(), &state).await;
        drop(state);
        tokio::spawn(run_prediction(store, session, service_revision));
    }
    Ok(service_revision)
}

async fn run_prediction(store: Arc<SessionStore>, session: Arc<Session>, service_revision: u64) {
    let request = PredictionRequest {
        session_id: session.id.clone(),
        revision: service_revision,
        mode: Some(PredictionMode::Hybrid),
        max_candidates: Some(5),
        max_tokens: Some(8),
    };
    match store.model.predict(&request).await {
        Ok(response) if response.status == "ready" => {
            let mut state = session.state.lock().await;
            if session
                .service_revision
                .load(std::sync::atomic::Ordering::SeqCst)
                != service_revision
            {
                return;
            }
            if !state.input.is_empty() {
                return;
            }
            state.predictions = response.candidates;
            state.model_pending = false;
            session.broadcast(store.as_ref(), &state).await;
        }
        Ok(response) => {
            debug!(status = %response.status, "prediction fell back");
            clear_prediction_pending(&store, &session, service_revision).await;
        }
        Err(error) => {
            warn!(%error, "prediction failed; keeping dictionary state");
            clear_prediction_pending(&store, &session, service_revision).await;
        }
    }
}

/// The model reordered (or dropped) candidates; keep the dictionary base
/// scores and adopt the model order, preserving selection by stable id.
fn merge_candidates(state: &mut InputState, model: &[ModelCandidate]) {
    let by_id = state
        .candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let selected = state.candidates.get(state.selected_index).cloned();
    let merged = model
        .iter()
        .filter_map(|candidate| by_id.get(candidate.id.as_str()).cloned())
        .collect::<Vec<_>>();
    if merged.is_empty() {
        return;
    }
    state.candidates = merged;
    state.selected_index = selected
        .and_then(|selected| {
            state
                .candidates
                .iter()
                .position(|candidate| candidate.id == selected.id)
        })
        .unwrap_or(state.selected_index);
    state.normalize_selection();
}

async fn clear_candidate_pending(store: &SessionStore, session: &Session, revision: u64) {
    let mut state = session.state.lock().await;
    if state.revision != revision {
        return;
    }
    if state.model_pending {
        state.model_pending = false;
        session.broadcast(store, &state).await;
    }
}

async fn clear_prediction_pending(store: &SessionStore, session: &Session, service_revision: u64) {
    let mut state = session.state.lock().await;
    if session
        .service_revision
        .load(std::sync::atomic::Ordering::SeqCst)
        != service_revision
    {
        return;
    }
    if state.model_pending {
        state.model_pending = false;
        session.broadcast(store, &state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::merge_candidates;
    use ime_core::{Candidate, InputState};
    use model_protocol::ModelCandidate;

    #[test]
    fn merging_fewer_candidates_clamps_page_and_selection() {
        let mut state = InputState {
            candidates: (0..15)
                .map(|index| Candidate {
                    id: format!("d:{index}"),
                    text: format!("字{index}"),
                    preedit: "shi".to_string(),
                    consumedkeys: 3,
                    base_score: 1.0,
                    kind: "dictionary".to_string(),
                })
                .collect(),
            selected_index: 12,
            page: 1,
            ..InputState::default()
        };
        let model = (0..2)
            .map(|index| ModelCandidate {
                id: format!("d:{index}"),
                text: format!("字{index}"),
                preedit: "shi".to_string(),
                consumedkeys: 3,
                score: index as f32,
                kind: "llm_phrase".to_string(),
            })
            .collect::<Vec<_>>();

        merge_candidates(&mut state, &model);

        assert_eq!(state.candidates.len(), 2);
        assert_eq!(state.page, 0);
        assert_eq!(state.selected_index, 1);
    }
}

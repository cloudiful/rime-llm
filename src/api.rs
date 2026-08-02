use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
pub use model_protocol::{
    CandidatePath, CandidatesRequest, CandidatesResponse, CommitRequest, PredictionMode,
    PredictionRequest, PredictionResponse, ResetRequest, ResetResponse,
};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::{
    config::Settings, model::ModelRuntime, predict_queue::PredictionCoordinator,
    session::SessionStore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthResponse {
    status: &'static str,
    model: String,
    model_file: String,
    device: String,
}

#[derive(Debug, Clone, Serialize)]
struct StatsResponse {
    candidate_requests: u64,
    commits: u64,
    resets: u64,
    model_requests: u64,
    model_successes: u64,
    fallbacks: u64,
    timeouts: u64,
    prediction_requests: u64,
    prediction_successes: u64,
    prediction_stale: u64,
}

#[derive(Default)]
pub(crate) struct Stats {
    candidate_requests: AtomicU64,
    commits: AtomicU64,
    resets: AtomicU64,
    model_requests: AtomicU64,
    model_successes: AtomicU64,
    fallbacks: AtomicU64,
    timeouts: AtomicU64,
    prediction_requests: AtomicU64,
    prediction_successes: AtomicU64,
    prediction_stale: AtomicU64,
}

impl Stats {
    pub(crate) fn record_prediction_request(&self) {
        self.prediction_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_prediction_success(&self) {
        self.prediction_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_prediction_stale(&self) {
        self.prediction_stale.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_model_request(&self) {
        self.model_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_model_success(&self) {
        self.model_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_fallback(&self) {
        self.fallbacks.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_timeout(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> StatsResponse {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        StatsResponse {
            candidate_requests: load(&self.candidate_requests),
            commits: load(&self.commits),
            resets: load(&self.resets),
            model_requests: load(&self.model_requests),
            model_successes: load(&self.model_successes),
            fallbacks: load(&self.fallbacks),
            timeouts: load(&self.timeouts),
            prediction_requests: load(&self.prediction_requests),
            prediction_successes: load(&self.prediction_successes),
            prediction_stale: load(&self.prediction_stale),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) settings: Settings,
    pub(crate) runtime: Arc<ModelRuntime>,
    pub(crate) sessions: SessionStore,
    pub(crate) predictions: Arc<PredictionCoordinator>,
    pub(crate) stats: Arc<Stats>,
}

impl AppState {
    pub fn new(settings: Settings, runtime: ModelRuntime) -> Self {
        let runtime = Arc::new(runtime);
        let sessions = SessionStore::new(settings.max_context_chars);
        let stats = Arc::new(Stats::default());
        let predictions = Arc::new(PredictionCoordinator::new(
            runtime.clone(),
            sessions.clone(),
            stats.clone(),
            settings.prediction_mode,
            settings.prediction_max_candidates,
            settings.prediction_max_tokens,
            settings.prediction_timeout_ms,
        ));
        Self {
            sessions,
            settings,
            runtime,
            predictions,
            stats,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/candidates", post(candidates))
        .route("/predict", post(predict))
        .route("/commit", post(commit))
        .route("/reset", post(reset))
        .route("/stats", get(stats))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        model: state.settings.model_repo.clone(),
        model_file: state.runtime.model_file.clone(),
        device: state.runtime.device.clone(),
    })
}

async fn stats(State(state): State<AppState>) -> Json<StatsResponse> {
    Json(state.stats.snapshot())
}

async fn candidates(
    State(state): State<AppState>,
    Json(request): Json<CandidatesRequest>,
) -> Json<CandidatesResponse> {
    state
        .stats
        .candidate_requests
        .fetch_add(1, Ordering::Relaxed);
    let started = std::time::Instant::now();
    let input = normalize_input(&request.input);
    let max_candidates = request
        .max_candidates
        .unwrap_or(state.settings.max_candidates)
        .clamp(1, state.settings.max_candidates);
    if request.session_id.trim().is_empty() || input.is_empty() {
        state.stats.fallbacks.fetch_add(1, Ordering::Relaxed);
        return Json(fallback_response(started, "invalid_request"));
    }

    let context = state.sessions.context(request.session_id.trim()).await;
    state.stats.record_model_request();
    match timeout(
        Duration::from_millis(state.settings.max_wait_ms),
        state
            .runtime
            .candidates(&context, &input, &request.paths, max_candidates),
    )
    .await
    {
        Ok(Ok(candidates)) if !candidates.is_empty() => {
            state.stats.record_model_success();
            Json(CandidatesResponse {
                status: "ready".to_string(),
                candidates,
                source: "model".to_string(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }
        Ok(Ok(_)) => {
            state.stats.record_fallback();
            Json(fallback_response(started, "no_match"))
        }
        Ok(Err(error)) => {
            state.stats.record_fallback();
            tracing::debug!(error = %error, "model candidate request failed");
            Json(fallback_response(started, "model_error"))
        }
        Err(_) => {
            state.stats.record_timeout();
            state.stats.record_fallback();
            Json(fallback_response(started, "timeout"))
        }
    }
}

async fn predict(
    State(state): State<AppState>,
    Json(request): Json<PredictionRequest>,
) -> Json<PredictionResponse> {
    state.stats.record_prediction_request();
    let started = std::time::Instant::now();
    let session_id = request.session_id.trim();
    let max_candidates = request
        .max_candidates
        .unwrap_or(state.settings.prediction_max_candidates)
        .clamp(1, state.settings.prediction_max_candidates);
    let max_tokens = request
        .max_tokens
        .unwrap_or(state.settings.prediction_max_tokens)
        .clamp(1, state.settings.prediction_max_tokens);
    if session_id.is_empty() {
        state.stats.record_fallback();
        return Json(prediction_fallback_response(
            started,
            request.revision,
            "invalid_request",
        ));
    }

    let revision = state.sessions.revision(session_id).await;
    if revision != request.revision {
        state.stats.record_prediction_stale();
        return Json(prediction_stale_response(started, revision));
    }

    let context = state.sessions.context(session_id).await;
    let request = PredictionRequest {
        session_id: request.session_id,
        revision: request.revision,
        mode: request.mode,
        max_candidates: Some(max_candidates),
        max_tokens: Some(max_tokens),
    };
    let receiver = state.predictions.submit(request, context).await;
    match timeout(
        Duration::from_millis(state.settings.prediction_timeout_ms),
        receiver,
    )
    .await
    {
        Ok(Ok(response)) => Json(response),
        Ok(Err(_)) | Err(_) => {
            state.stats.record_timeout();
            state.stats.record_fallback();
            Json(prediction_fallback_response(started, revision, "timeout"))
        }
    }
}

async fn commit(
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Json<serde_json::Value> {
    if !request.session_id.trim().is_empty() && !request.text.is_empty() {
        let revision = state
            .sessions
            .commit(request.session_id.trim(), &request.text)
            .await;
        state.stats.commits.fetch_add(1, Ordering::Relaxed);
        return Json(serde_json::json!({"status": "ok", "revision": revision}));
    }
    Json(serde_json::json!({
        "status": "ok",
        "revision": state.sessions.revision(request.session_id.trim()).await
    }))
}

async fn reset(
    State(state): State<AppState>,
    Json(request): Json<ResetRequest>,
) -> Json<ResetResponse> {
    let revision = state.sessions.reset(request.session_id.trim()).await;
    state.stats.resets.fetch_add(1, Ordering::Relaxed);
    Json(ResetResponse {
        status: "ok".to_string(),
        revision,
    })
}

fn fallback_response(started: std::time::Instant, source: &str) -> CandidatesResponse {
    CandidatesResponse {
        status: "fallback".to_string(),
        candidates: Vec::new(),
        source: source.to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn prediction_fallback_response(
    started: std::time::Instant,
    revision: u64,
    source: &str,
) -> PredictionResponse {
    PredictionResponse {
        status: "fallback".to_string(),
        revision,
        candidates: Vec::new(),
        source: source.to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn prediction_stale_response(started: std::time::Instant, revision: u64) -> PredictionResponse {
    PredictionResponse {
        status: "stale".to_string(),
        revision,
        candidates: Vec::new(),
        source: "stale".to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn normalize_input(input: &str) -> String {
    input
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_input_filters_pinyin_only() {
        assert_eq!(normalize_input("Ni Hao! 不如 bu-ru"), "nihaoburu");
    }
}

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
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::{
    config::Settings,
    model::{ModelCandidate, ModelRuntime},
    session::SessionStore,
};

#[derive(Debug, Clone, Deserialize)]
pub struct CandidatesRequest {
    pub session_id: String,
    pub input: String,
    #[serde(default)]
    pub max_candidates: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidatesResponse {
    pub status: String,
    pub candidates: Vec<ModelCandidate>,
    pub source: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitRequest {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResetRequest {
    pub session_id: String,
}

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
}

#[derive(Default)]
pub struct Stats {
    candidate_requests: AtomicU64,
    commits: AtomicU64,
    resets: AtomicU64,
    model_requests: AtomicU64,
    model_successes: AtomicU64,
    fallbacks: AtomicU64,
    timeouts: AtomicU64,
}

impl Stats {
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
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub settings: Settings,
    pub runtime: Arc<ModelRuntime>,
    pub sessions: SessionStore,
    pub stats: Arc<Stats>,
}

impl AppState {
    pub fn new(settings: Settings, runtime: ModelRuntime) -> Self {
        Self {
            sessions: SessionStore::new(settings.max_context_chars),
            settings,
            runtime: Arc::new(runtime),
            stats: Arc::new(Stats::default()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/candidates", post(candidates))
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
    state.stats.model_requests.fetch_add(1, Ordering::Relaxed);
    match timeout(
        Duration::from_millis(state.settings.max_wait_ms),
        state.runtime.candidates(&context, &input, max_candidates),
    )
    .await
    {
        Ok(Ok(candidates)) if !candidates.is_empty() => {
            state.stats.model_successes.fetch_add(1, Ordering::Relaxed);
            Json(CandidatesResponse {
                status: "ready".to_string(),
                candidates,
                source: "model".to_string(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }
        Ok(Ok(_)) => {
            state.stats.fallbacks.fetch_add(1, Ordering::Relaxed);
            Json(fallback_response(started, "no_match"))
        }
        Ok(Err(error)) => {
            state.stats.fallbacks.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(error = %error, "model candidate request failed");
            Json(fallback_response(started, "model_error"))
        }
        Err(_) => {
            state.stats.timeouts.fetch_add(1, Ordering::Relaxed);
            state.stats.fallbacks.fetch_add(1, Ordering::Relaxed);
            Json(fallback_response(started, "timeout"))
        }
    }
}

async fn commit(
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Json<serde_json::Value> {
    if !request.session_id.trim().is_empty() && !request.text.is_empty() {
        state
            .sessions
            .commit(request.session_id.trim(), &request.text)
            .await;
        state.stats.commits.fetch_add(1, Ordering::Relaxed);
    }
    Json(serde_json::json!({"status": "ok"}))
}

async fn reset(
    State(state): State<AppState>,
    Json(request): Json<ResetRequest>,
) -> Json<serde_json::Value> {
    state.sessions.reset(request.session_id.trim()).await;
    state.stats.resets.fetch_add(1, Ordering::Relaxed);
    Json(serde_json::json!({"status": "ok"}))
}

fn fallback_response(started: std::time::Instant, source: &str) -> CandidatesResponse {
    CandidatesResponse {
        status: "fallback".to_string(),
        candidates: Vec::new(),
        source: source.to_string(),
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

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
    predict_queue::PredictionCoordinator,
    prediction::{PredictionCandidate, PredictionMode},
    session::SessionStore,
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CandidatePath {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub preedit: String,
    #[serde(default)]
    pub consumedkeys: usize,
    #[serde(default)]
    pub base_score: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CandidatesRequest {
    pub session_id: String,
    pub input: String,
    #[serde(default)]
    pub max_candidates: Option<usize>,
    #[serde(default)]
    pub paths: Vec<CandidatePath>,
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
pub struct PredictionRequest {
    pub session_id: String,
    pub revision: u64,
    #[serde(default)]
    pub mode: Option<PredictionMode>,
    #[serde(default)]
    pub max_candidates: Option<usize>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionResponse {
    pub status: String,
    pub revision: u64,
    pub candidates: Vec<PredictionCandidate>,
    pub source: String,
    pub elapsed_ms: u64,
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
) -> Json<serde_json::Value> {
    let revision = state.sessions.reset(request.session_id.trim()).await;
    state.stats.resets.fetch_add(1, Ordering::Relaxed);
    Json(serde_json::json!({"status": "ok", "revision": revision}))
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

    fn path(id: &str, text: &str, base: f32, consumedkeys: usize) -> CandidatePath {
        CandidatePath {
            id: id.into(),
            text: text.into(),
            preedit: String::new(),
            consumedkeys,
            base_score: base,
        }
    }

    #[test]
    fn candidates_request_accepts_paths_with_default_optional_fields() {
        let json = r#"{
            "session_id": "abc",
            "input": "buru",
            "max_candidates": 4,
            "paths": [
                {"id": "n0", "text": "不如", "preedit": "bu ru", "consumedkeys": 4, "base_score": 1.5},
                {"id": "n1", "text": "不入", "base_score": -0.2}
            ]
        }"#;
        let request: CandidatesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.session_id, "abc");
        assert_eq!(request.paths.len(), 2);
        assert_eq!(request.paths[0].text, "不如");
        assert_eq!(request.paths[0].consumedkeys, 4);
        assert_eq!(request.paths[0].base_score, 1.5);
        assert_eq!(request.paths[1].preedit, "");
        assert_eq!(request.paths[1].consumedkeys, 0);
        assert_eq!(request.paths[1].base_score, -0.2);
    }

    #[test]
    fn candidates_request_without_paths_defaults_to_empty_list() {
        let json = r#"{"session_id":"abc","input":"buru"}"#;
        let request: CandidatesRequest = serde_json::from_str(json).unwrap();
        assert!(request.paths.is_empty());
        assert!(request.max_candidates.is_none());
    }

    #[test]
    fn candidate_path_serialization_round_trip() {
        let original = CandidatePath {
            id: "x".into(),
            text: "你好".into(),
            preedit: "ni hao".into(),
            consumedkeys: 5,
            base_score: 0.75,
        };
        let value = serde_json::to_value(&original).unwrap();
        assert_eq!(value["id"], "x");
        assert_eq!(value["text"], "你好");
        assert_eq!(value["preedit"], "ni hao");
        assert_eq!(value["consumedkeys"], 5);
        assert_eq!(value["base_score"], 0.75);
        let decoded: CandidatePath = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn normalize_input_filters_pinyin_only() {
        assert_eq!(normalize_input("Ni Hao! 不如 bu-ru"), "nihaoburu");
    }

    #[test]
    fn path_helper_builds_expected_shape() {
        let p = path("n2", "不如", 0.0, 4);
        assert_eq!(p.id, "n2");
        assert_eq!(p.text, "不如");
        assert_eq!(p.consumedkeys, 4);
    }

    #[test]
    fn candidates_response_status_round_trip() {
        let response = CandidatesResponse {
            status: "ready".into(),
            candidates: vec![ModelCandidate {
                id: "m0".into(),
                text: "不如".into(),
                preedit: "bu ru".into(),
                consumedkeys: 4,
                score: -0.42,
                kind: "llm_phrase".into(),
            }],
            source: "model".into(),
            elapsed_ms: 7,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["status"], "ready");
        assert_eq!(value["candidates"][0]["type"], "llm_phrase");
        assert_eq!(value["candidates"][0]["consumedkeys"], 4);
        let decoded: CandidatesResponse = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, response);
    }
}

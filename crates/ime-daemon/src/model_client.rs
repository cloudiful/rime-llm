//! Client for the rime-llm model service, plus a scriptable mock for tests.
//! Enum dispatch keeps the real and mock clients swappable without `dyn`.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use model_protocol::{
    CandidatesRequest, CandidatesResponse, CommitRequest, PredictionRequest, PredictionResponse,
    ResetRequest, ResetResponse,
};
use tokio::sync::{mpsc, Mutex};

pub enum ModelClient {
    Reqwest(ReqwestModelApi),
    Mock(Arc<MockModelApi>),
}

impl ModelClient {
    pub async fn candidates(&self, request: &CandidatesRequest) -> Result<CandidatesResponse> {
        match self {
            Self::Reqwest(inner) => inner.candidates(request).await,
            Self::Mock(inner) => inner.candidates(request).await,
        }
    }

    pub async fn commit(&self, request: &CommitRequest) -> Result<u64> {
        match self {
            Self::Reqwest(inner) => inner.commit(request).await,
            Self::Mock(inner) => inner.commit(request).await,
        }
    }

    pub async fn predict(&self, request: &PredictionRequest) -> Result<PredictionResponse> {
        match self {
            Self::Reqwest(inner) => inner.predict(request).await,
            Self::Mock(inner) => inner.predict(request).await,
        }
    }

    pub async fn reset(&self, request: &ResetRequest) -> Result<ResetResponse> {
        match self {
            Self::Reqwest(inner) => inner.reset(request).await,
            Self::Mock(inner) => inner.reset(request).await,
        }
    }
}

pub struct ReqwestModelApi {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestModelApi {
    pub fn new(base_url: String, timeout_ms: u64) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("build model service HTTP client")?;
        Ok(Self { client, base_url })
    }
}

impl ReqwestModelApi {
    async fn candidates(&self, request: &CandidatesRequest) -> Result<CandidatesResponse> {
        self.client
            .post(format!("{}/candidates", self.base_url))
            .json(request)
            .send()
            .await
            .context("model candidate request failed")?
            .error_for_status()
            .context("model candidate request rejected")?
            .json()
            .await
            .context("invalid model candidate response")
    }

    async fn commit(&self, request: &CommitRequest) -> Result<u64> {
        let value: serde_json::Value = self
            .client
            .post(format!("{}/commit", self.base_url))
            .json(request)
            .send()
            .await
            .context("model commit request failed")?
            .error_for_status()
            .context("model commit request rejected")?
            .json()
            .await
            .context("invalid model commit response")?;
        value
            .get("revision")
            .and_then(|revision| revision.as_u64())
            .context("model commit response has no revision")
    }

    pub async fn predict(&self, request: &PredictionRequest) -> Result<PredictionResponse> {
        self.client
            .post(format!("{}/predict", self.base_url))
            .json(request)
            .send()
            .await
            .context("model prediction request failed")?
            .error_for_status()
            .context("model prediction request rejected")?
            .json()
            .await
            .context("invalid model prediction response")
    }

    async fn reset(&self, request: &ResetRequest) -> Result<ResetResponse> {
        self.client
            .post(format!("{}/reset", self.base_url))
            .json(request)
            .send()
            .await
            .context("model reset request failed")?
            .error_for_status()
            .context("model reset request rejected")?
            .json()
            .await
            .context("invalid model reset response")
    }
}

/// Test double: each `candidates`/`predict` call waits for the test to push
/// a response, so stale-revision races can be staged deterministically.
pub struct MockModelApi {
    candidates_tx: mpsc::UnboundedSender<CandidatesResponse>,
    candidates_rx: Mutex<mpsc::UnboundedReceiver<CandidatesResponse>>,
    predict_tx: mpsc::UnboundedSender<PredictionResponse>,
    predict_rx: Mutex<mpsc::UnboundedReceiver<PredictionResponse>>,
    seen_candidates: Arc<Mutex<Vec<CandidatesRequest>>>,
    commit_revision: AtomicU64,
    reset_requests: Arc<Mutex<Vec<ResetRequest>>>,
    reset_failed: Arc<AtomicBool>,
}

impl MockModelApi {
    pub fn new() -> (Arc<Self>, MockDriver) {
        let (candidates_tx, candidates_rx) = mpsc::unbounded_channel();
        let (predict_tx, predict_rx) = mpsc::unbounded_channel();
        let mock = Arc::new(Self {
            candidates_tx,
            candidates_rx: Mutex::new(candidates_rx),
            predict_tx,
            predict_rx: Mutex::new(predict_rx),
            seen_candidates: Arc::new(Mutex::new(Vec::new())),
            commit_revision: AtomicU64::new(0),
            reset_requests: Arc::new(Mutex::new(Vec::new())),
            reset_failed: Arc::new(AtomicBool::new(false)),
        });
        let driver = MockDriver {
            candidates_tx: mock.candidates_tx.clone(),
            predict_tx: mock.predict_tx.clone(),
            seen_candidates: mock.seen_candidates.clone(),
            reset_requests: mock.reset_requests.clone(),
            reset_failed: mock.reset_failed.clone(),
        };
        (mock, driver)
    }
}

#[derive(Clone)]
pub struct MockDriver {
    candidates_tx: mpsc::UnboundedSender<CandidatesResponse>,
    predict_tx: mpsc::UnboundedSender<PredictionResponse>,
    seen_candidates: Arc<Mutex<Vec<CandidatesRequest>>>,
    reset_requests: Arc<Mutex<Vec<ResetRequest>>>,
    reset_failed: Arc<AtomicBool>,
}

impl MockDriver {
    pub fn respond_candidates(&self, response: CandidatesResponse) {
        let _ = self.candidates_tx.send(response);
    }

    pub fn respond_predictions(&self, response: PredictionResponse) {
        let _ = self.predict_tx.send(response);
    }

    pub async fn seen_candidates(&self) -> Vec<CandidatesRequest> {
        self.seen_candidates.lock().await.clone()
    }

    pub async fn reset_requests(&self) -> Vec<ResetRequest> {
        self.reset_requests.lock().await.clone()
    }

    pub fn set_reset_failure(&self, failed: bool) {
        self.reset_failed.store(failed, Ordering::SeqCst);
    }
}

impl MockModelApi {
    async fn candidates(&self, request: &CandidatesRequest) -> Result<CandidatesResponse> {
        self.seen_candidates.lock().await.push(request.clone());
        self.candidates_rx
            .lock()
            .await
            .recv()
            .await
            .context("no mock candidate response queued")
    }

    async fn commit(&self, _request: &CommitRequest) -> Result<u64> {
        Ok(self.commit_revision.fetch_add(1, Ordering::SeqCst) + 1)
    }

    async fn predict(&self, _request: &PredictionRequest) -> Result<PredictionResponse> {
        self.predict_rx
            .lock()
            .await
            .recv()
            .await
            .context("no mock prediction response queued")
    }

    async fn reset(&self, request: &ResetRequest) -> Result<ResetResponse> {
        self.reset_requests.lock().await.push(request.clone());
        if self.reset_failed.load(Ordering::SeqCst) {
            anyhow::bail!("mock reset failed");
        }
        Ok(ResetResponse {
            status: "ok".to_string(),
            revision: self.commit_revision.fetch_add(1, Ordering::SeqCst) + 1,
        })
    }
}

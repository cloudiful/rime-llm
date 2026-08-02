use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use tokio::sync::{oneshot, Mutex, Notify};

use crate::{
    api::{PredictionRequest, PredictionResponse, Stats},
    model::ModelRuntime,
    prediction::{PredictionCandidate, PredictionMode, PredictionResult},
    session::SessionStore,
};

#[derive(Clone)]
pub struct PredictionCoordinator {
    backend: Arc<dyn PredictionBackend>,
    sessions: SessionStore,
    stats: Arc<Stats>,
    default_mode: PredictionMode,
    max_candidates: usize,
    max_tokens: usize,
    state: Arc<Mutex<State>>,
    wakeup: Arc<Notify>,
}

pub(crate) trait PredictionBackend: Send + Sync {
    fn predict<'a>(
        &'a self,
        context: &'a [String],
        mode: PredictionMode,
        max_candidates: usize,
        max_tokens: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PredictionResult>> + Send + 'a>>;
}

impl PredictionBackend for ModelRuntime {
    fn predict<'a>(
        &'a self,
        context: &'a [String],
        mode: PredictionMode,
        max_candidates: usize,
        max_tokens: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PredictionResult>> + Send + 'a>> {
        Box::pin(ModelRuntime::predict(
            self,
            context,
            mode,
            max_candidates,
            max_tokens,
        ))
    }
}

struct State {
    started: bool,
    pending: Option<QueuedJob>,
}

struct QueuedJob {
    request: PredictionRequest,
    context: Vec<String>,
    started: Instant,
    responder: oneshot::Sender<PredictionResponse>,
}

impl PredictionCoordinator {
    pub(crate) fn new(
        backend: Arc<dyn PredictionBackend>,
        sessions: SessionStore,
        stats: Arc<Stats>,
        default_mode: PredictionMode,
        max_candidates: usize,
        max_tokens: usize,
        _timeout_ms: u64,
    ) -> Self {
        Self {
            backend,
            sessions,
            stats,
            default_mode,
            max_candidates: max_candidates.clamp(1, 16),
            max_tokens: max_tokens.clamp(1, 32),
            state: Arc::new(Mutex::new(State {
                started: false,
                pending: None,
            })),
            wakeup: Arc::new(Notify::new()),
        }
    }

    pub async fn submit(
        &self,
        request: PredictionRequest,
        context: Vec<String>,
    ) -> oneshot::Receiver<PredictionResponse> {
        let (sender, receiver) = oneshot::channel();
        let job = QueuedJob {
            request,
            context,
            started: Instant::now(),
            responder: sender,
        };

        let replaced = {
            let mut state = self.state.lock().await;
            let replaced = state.pending.replace(job);
            if !state.started {
                state.started = true;
                let worker = self.clone();
                tokio::spawn(async move {
                    worker.run().await;
                });
            }
            replaced
        };

        if let Some(old) = replaced {
            self.stats.record_prediction_stale();
            let _ = old.responder.send(stale_response(
                old.request.revision,
                old.started.elapsed().as_millis() as u64,
            ));
        }

        self.wakeup.notify_one();
        receiver
    }

    async fn run(self) {
        loop {
            let notified = self.wakeup.notified();
            let job = {
                let mut state = self.state.lock().await;
                state.pending.take()
            };

            if let Some(job) = job {
                self.process(job).await;
                continue;
            }

            notified.await;
        }
    }

    async fn process(&self, job: QueuedJob) {
        self.stats.record_model_request();

        let revision = self.sessions.revision(&job.request.session_id).await;
        if revision != job.request.revision {
            self.stats.record_prediction_stale();
            let _ = job.responder.send(stale_response(
                revision,
                job.started.elapsed().as_millis() as u64,
            ));
            return;
        }

        let mode = job.request.mode.unwrap_or(self.default_mode);
        let max_candidates = job
            .request
            .max_candidates
            .unwrap_or(self.max_candidates)
            .clamp(1, self.max_candidates);
        let max_tokens = job
            .request
            .max_tokens
            .unwrap_or(self.max_tokens)
            .clamp(1, self.max_tokens);
        // The HTTP handler owns the caller-facing timeout. The coordinator
        // must keep awaiting the active backend job so a timed-out request
        // cannot detach a blocking inference task from the single worker.
        match self
            .backend
            .predict(&job.context, mode, max_candidates, max_tokens)
            .await
        {
            Ok(result) if !result.candidates.is_empty() => {
                let latest_revision = self.sessions.revision(&job.request.session_id).await;
                if latest_revision != job.request.revision {
                    self.stats.record_prediction_stale();
                    let _ = job.responder.send(stale_response(
                        latest_revision,
                        job.started.elapsed().as_millis() as u64,
                    ));
                    return;
                }

                self.stats.record_model_success();
                self.stats.record_prediction_success();
                let _ = job.responder.send(PredictionResponse {
                    status: "ready".to_string(),
                    revision: job.request.revision,
                    candidates: result.candidates,
                    source: result.source,
                    elapsed_ms: job.started.elapsed().as_millis() as u64,
                });
            }
            Ok(_) => {
                self.stats.record_fallback();
                let _ = job.responder.send(PredictionResponse {
                    status: "fallback".to_string(),
                    revision: job.request.revision,
                    candidates: Vec::new(),
                    source: "no_match".to_string(),
                    elapsed_ms: job.started.elapsed().as_millis() as u64,
                });
            }
            Err(error) => {
                tracing::debug!(error = %error, "model prediction request failed");
                self.stats.record_fallback();
                let _ = job.responder.send(PredictionResponse {
                    status: "fallback".to_string(),
                    revision: job.request.revision,
                    candidates: Vec::new(),
                    source: "model_error".to_string(),
                    elapsed_ms: job.started.elapsed().as_millis() as u64,
                });
            }
        }
    }
}

fn stale_response(revision: u64, elapsed_ms: u64) -> PredictionResponse {
    PredictionResponse {
        status: "stale".to_string(),
        revision,
        candidates: Vec::<PredictionCandidate>::new(),
        source: "stale".to_string(),
        elapsed_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    struct FakeBackend {
        started: AtomicUsize,
        entered: Notify,
        release: Notify,
    }

    impl PredictionBackend for FakeBackend {
        fn predict<'a>(
            &'a self,
            _context: &'a [String],
            _mode: PredictionMode,
            _max_candidates: usize,
            _max_tokens: usize,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PredictionResult>> + Send + 'a>> {
            Box::pin(async move {
                let sequence = self.started.fetch_add(1, Ordering::SeqCst) + 1;
                self.entered.notify_one();
                self.release.notified().await;
                Ok(PredictionResult {
                    candidates: vec![PredictionCandidate {
                        id: format!("fake-{sequence}"),
                        text: format!("候选{sequence}"),
                        score: 1.0,
                        kind: "test".to_string(),
                    }],
                    source: "test".to_string(),
                })
            })
        }
    }

    async fn wait_until_started(backend: &FakeBackend, expected: usize) {
        while backend.started.load(Ordering::SeqCst) < expected {
            backend.entered.notified().await;
        }
    }

    fn request(revision: u64, session_id: &str) -> PredictionRequest {
        PredictionRequest {
            session_id: session_id.to_string(),
            revision,
            mode: Some(PredictionMode::Free),
            max_candidates: Some(5),
            max_tokens: Some(1),
        }
    }

    #[tokio::test]
    async fn keeps_one_running_job_and_only_the_latest_pending_job() {
        let backend = Arc::new(FakeBackend {
            started: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let sessions = SessionStore::new(20);
        assert_eq!(sessions.commit("s", "甲").await, 1);
        let stats = Arc::new(Stats::default());
        let coordinator = PredictionCoordinator::new(
            backend.clone(),
            sessions,
            stats,
            PredictionMode::Free,
            5,
            8,
            1_000,
        );

        let first = coordinator
            .submit(request(1, "s"), vec!["甲".to_string()])
            .await;
        wait_until_started(&backend, 1).await;
        let second = coordinator
            .submit(request(1, "s"), vec!["甲".to_string()])
            .await;
        let third = coordinator
            .submit(request(1, "s"), vec!["甲".to_string()])
            .await;

        let second_response = tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .expect("replaced request should complete")
            .expect("stale response should be sent");
        assert_eq!(second_response.status, "stale");
        assert_eq!(backend.started.load(Ordering::SeqCst), 1);

        backend.release.notify_one();
        let first_response = first.await.expect("first response should be sent");
        assert_eq!(first_response.status, "ready");
        wait_until_started(&backend, 2).await;
        backend.release.notify_one();
        let third_response = third.await.expect("latest response should be sent");
        assert_eq!(third_response.status, "ready");
        assert_eq!(backend.started.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dropping_timed_out_caller_does_not_detach_active_job() {
        let backend = Arc::new(FakeBackend {
            started: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let sessions = SessionStore::new(20);
        assert_eq!(sessions.commit("s", "甲").await, 1);
        let stats = Arc::new(Stats::default());
        let coordinator = PredictionCoordinator::new(
            backend.clone(),
            sessions,
            stats,
            PredictionMode::Free,
            5,
            8,
            1,
        );

        let first = coordinator
            .submit(request(1, "s"), vec!["甲".to_string()])
            .await;
        wait_until_started(&backend, 1).await;
        drop(first);

        let latest = coordinator
            .submit(request(1, "s"), vec!["甲".to_string()])
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(backend.started.load(Ordering::SeqCst), 1);

        backend.release.notify_one();
        wait_until_started(&backend, 2).await;
        backend.release.notify_one();
        let response = latest.await.expect("latest response should be sent");
        assert_eq!(response.status, "ready");
        assert_eq!(backend.started.load(Ordering::SeqCst), 2);
    }
}

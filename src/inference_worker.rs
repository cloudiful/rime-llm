//! Single-owner executor for llama.cpp inference.
//!
//! Callers may time out while waiting for a result, but the worker keeps the
//! active job attached to this queue. That bounds the number of blocking
//! tasks and lets newer work replace only the pending job.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use model_protocol::{CandidatePath, ModelCandidate};
use tokio::sync::{oneshot, Mutex as AsyncMutex, Notify};
use tracing::debug;

use crate::{llama_engine::LlamaEngine, model::score_candidates};

pub(crate) struct InferenceWorker {
    engine: Arc<Mutex<LlamaEngine>>,
    state: Arc<AsyncMutex<State>>,
    wakeup: Arc<Notify>,
}

struct State {
    started: bool,
    pending: Option<Job>,
}

enum Job {
    Candidates {
        prompt: String,
        paths: Vec<CandidatePath>,
        max_candidates: usize,
        responder: oneshot::Sender<Result<Vec<ModelCandidate>>>,
    },
    Generate {
        prompt: String,
        max_tokens: usize,
        responder: oneshot::Sender<Result<String>>,
    },
}

impl Job {
    fn superseded(self) {
        self.fail("inference request superseded");
    }

    fn fail(self, message: &str) {
        match self {
            Self::Candidates { responder, .. } => {
                let _ = responder.send(Err(anyhow!(message.to_string())));
            }
            Self::Generate { responder, .. } => {
                let _ = responder.send(Err(anyhow!(message.to_string())));
            }
        }
    }
}

impl InferenceWorker {
    pub(crate) fn new(engine: Arc<Mutex<LlamaEngine>>) -> Arc<Self> {
        Arc::new(Self {
            engine,
            state: Arc::new(AsyncMutex::new(State {
                started: false,
                pending: None,
            })),
            wakeup: Arc::new(Notify::new()),
        })
    }

    pub(crate) async fn candidates(
        &self,
        prompt: String,
        paths: Vec<CandidatePath>,
        max_candidates: usize,
    ) -> Result<Vec<ModelCandidate>> {
        let (sender, receiver) = oneshot::channel();
        self.enqueue(Job::Candidates {
            prompt,
            paths,
            max_candidates,
            responder: sender,
        })
        .await;
        receiver.await.context("inference worker stopped")?
    }

    pub(crate) async fn generate(&self, prompt: String, max_tokens: usize) -> Result<String> {
        let (sender, receiver) = oneshot::channel();
        self.enqueue(Job::Generate {
            prompt,
            max_tokens,
            responder: sender,
        })
        .await;
        receiver.await.context("inference worker stopped")?
    }

    async fn enqueue(&self, job: Job) {
        let replaced = {
            let mut state = self.state.lock().await;
            let replaced = state.pending.replace(job);
            if !state.started {
                state.started = true;
                let worker = Arc::new(Self {
                    engine: Arc::clone(&self.engine),
                    state: Arc::clone(&self.state),
                    wakeup: Arc::clone(&self.wakeup),
                });
                tokio::spawn(async move {
                    worker.run().await;
                });
            }
            replaced
        };

        if let Some(replaced) = replaced {
            replaced.superseded();
        }
        self.wakeup.notify_one();
    }

    async fn run(self: Arc<Self>) {
        loop {
            let notified = self.wakeup.notified();
            let job = self.state.lock().await.pending.take();
            if let Some(job) = job {
                self.execute(job).await;
                continue;
            }
            notified.await;
        }
    }

    async fn execute(&self, job: Job) {
        let engine = Arc::clone(&self.engine);
        let result = tokio::task::spawn_blocking(move || match engine.lock() {
            Ok(engine) => match job {
                Job::Candidates {
                    prompt,
                    paths,
                    max_candidates,
                    responder,
                } => {
                    let result = score_candidates(&engine, &prompt, &paths, max_candidates);
                    let _ = responder.send(result);
                }
                Job::Generate {
                    prompt,
                    max_tokens,
                    responder,
                } => {
                    let result = engine
                        .generate(&prompt, max_tokens)
                        .context("generate next-word predictions");
                    let _ = responder.send(result);
                }
            },
            Err(_) => job.fail("llama.cpp inference lock is poisoned"),
        })
        .await;

        if let Err(error) = result {
            debug!(%error, "inference worker task failed");
        }
    }
}

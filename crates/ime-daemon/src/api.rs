//! Local HTTP/WebSocket API consumed by the Swift input method.

use std::{sync::Arc, time::Duration};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use ime_core::InputEvent;
use model_protocol::ResetRequest;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::{
    engine::{self, model_worker},
    session::{Session, SessionStore, StateSnapshot},
};

#[derive(Debug, Clone, Deserialize)]
pub struct KeyRequest {
    pub event: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Effects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyResponse {
    pub session_id: String,
    pub state: StateSnapshot,
    pub effects: Effects,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAckRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitAckResponse {
    pub ok: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub state: StateSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeleteSessionResponse {
    pub ok: bool,
    pub model_reset: bool,
    pub reset_retry_scheduled: bool,
}

pub fn router(store: Arc<SessionStore>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/{id}", delete(delete_session))
        .route("/v1/sessions/{id}/key", post(key))
        .route("/v1/sessions/{id}/commit-ack", post(commit_ack))
        .route("/v1/sessions/{id}/state", get(state))
        .route("/v1/sessions/{id}/events", get(events))
        .with_state(store)
}

async fn healthz() -> Json<OkResponse> {
    Json(OkResponse { ok: true })
}

async fn create_session(
    State(store): State<Arc<SessionStore>>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = store.create().await;
    tokio::spawn(model_worker(store.clone(), session.clone()));
    let snapshot = {
        let state = session.state.lock().await;
        session.snapshot(&store, &state).await
    };
    Ok(Json(SessionResponse {
        session_id: session.id.clone(),
        state: snapshot,
    }))
}

async fn delete_session(
    Path(id): Path<String>,
    State(store): State<Arc<SessionStore>>,
) -> Json<DeleteSessionResponse> {
    let removed = store.remove(&id).await;
    if !removed {
        return Json(DeleteSessionResponse {
            ok: true,
            model_reset: true,
            reset_retry_scheduled: false,
        });
    }

    let request = ResetRequest {
        session_id: id.clone(),
    };
    match timeout(Duration::from_millis(250), store.model.reset(&request)).await {
        Ok(Ok(response)) if response.status == "ok" => Json(DeleteSessionResponse {
            ok: true,
            model_reset: true,
            reset_retry_scheduled: false,
        }),
        Ok(Ok(response)) => {
            warn!(session_id = %id, status = %response.status, "model reset returned a non-ready status");
            schedule_reset_retry(store, id);
            Json(DeleteSessionResponse {
                ok: true,
                model_reset: false,
                reset_retry_scheduled: true,
            })
        }
        Ok(Err(error)) => {
            warn!(session_id = %id, %error, "model reset failed after session close");
            schedule_reset_retry(store, id);
            Json(DeleteSessionResponse {
                ok: true,
                model_reset: false,
                reset_retry_scheduled: true,
            })
        }
        Err(_) => {
            warn!(session_id = %id, "model reset timed out after session close");
            schedule_reset_retry(store, id);
            Json(DeleteSessionResponse {
                ok: true,
                model_reset: false,
                reset_retry_scheduled: true,
            })
        }
    }
}

fn schedule_reset_retry(store: Arc<SessionStore>, session_id: String) {
    tokio::spawn(async move {
        for delay in [Duration::from_millis(250), Duration::from_secs(1)] {
            tokio::time::sleep(delay).await;
            let request = ResetRequest {
                session_id: session_id.clone(),
            };
            match timeout(Duration::from_secs(1), store.model.reset(&request)).await {
                Ok(Ok(response)) if response.status == "ok" => return,
                Ok(Ok(response)) => {
                    debug!(session_id = %session_id, status = %response.status, "model reset retry returned a non-ready status");
                }
                Ok(Err(error)) => {
                    debug!(session_id = %session_id, %error, "model reset retry failed");
                }
                Err(_) => {
                    debug!(session_id = %session_id, "model reset retry timed out");
                }
            }
        }
        warn!(session_id = %session_id, "model reset retries exhausted");
    });
}

async fn key(
    Path(id): Path<String>,
    State(store): State<Arc<SessionStore>>,
    Json(request): Json<KeyRequest>,
) -> Result<Json<KeyResponse>, StatusCode> {
    let event = parse_event(&request).map_err(|message| {
        debug!(%message, "invalid key event");
        StatusCode::BAD_REQUEST
    })?;
    let session = store.get(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    let (snapshot, effect) =
        engine::handle_key(&store, &session, event)
            .await
            .map_err(|error| {
                debug!(%error, "key handling failed");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    Ok(Json(KeyResponse {
        session_id: session.id.clone(),
        state: snapshot,
        effects: effects_from(&effect),
    }))
}

async fn commit_ack(
    Path(id): Path<String>,
    State(store): State<Arc<SessionStore>>,
    Json(request): Json<CommitAckRequest>,
) -> Result<Json<CommitAckResponse>, StatusCode> {
    let session = store.get(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    let revision = engine::handle_commit_ack(store, session, &request.text)
        .await
        .map_err(|error| {
            debug!(%error, "commit ack failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(CommitAckResponse { ok: true, revision }))
}

async fn state(
    Path(id): Path<String>,
    State(store): State<Arc<SessionStore>>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = store.get(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    let snapshot = {
        let state = session.state.lock().await;
        session.snapshot(&store, &state).await
    };
    Ok(Json(SessionResponse {
        session_id: session.id.clone(),
        state: snapshot,
    }))
}

async fn events(
    Path(id): Path<String>,
    State(store): State<Arc<SessionStore>>,
    websocket: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, StatusCode> {
    let session = store.get(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    Ok(websocket.on_upgrade(move |socket| stream_events(socket, store, session)))
}

async fn stream_events(mut socket: WebSocket, store: Arc<SessionStore>, session: Arc<Session>) {
    // Subscribe before the initial snapshot so no broadcast can be lost
    // between connection setup and the first key event.
    let mut receiver = session.tx.subscribe();
    {
        let state = session.state.lock().await;
        let snapshot = session.snapshot(&store, &state).await;
        let Ok(payload) = serde_json::to_string(&snapshot) else {
            return;
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }
    loop {
        match receiver.recv().await {
            Ok(payload) => {
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        }
    }
}

fn parse_event(request: &KeyRequest) -> Result<InputEvent, &'static str> {
    match request.event.as_str() {
        "letter" => request
            .value
            .as_deref()
            .and_then(|value| value.chars().next())
            .map(|character| InputEvent::Letter(character.to_ascii_lowercase()))
            .ok_or("letter event requires a value"),
        "backspace" => Ok(InputEvent::Backspace),
        "delete" => Ok(InputEvent::Delete),
        "left" => Ok(InputEvent::Left),
        "right" => Ok(InputEvent::Right),
        "pageup" => Ok(InputEvent::PageUp),
        "pagedown" => Ok(InputEvent::PageDown),
        "space" => Ok(InputEvent::Space),
        "enter" => Ok(InputEvent::Enter),
        "escape" => Ok(InputEvent::Escape),
        "digit" => request
            .value
            .as_deref()
            .and_then(|value| value.parse::<u8>().ok())
            .map(InputEvent::Digit)
            .ok_or("digit event requires a number"),
        "select" => request
            .value
            .as_deref()
            .and_then(|value| value.parse::<usize>().ok())
            .map(InputEvent::SelectCandidate)
            .ok_or("select event requires an index"),
        _ => Err("unknown event"),
    }
}

fn effects_from(effect: &ime_core::Effect) -> Effects {
    match effect {
        ime_core::Effect::None => Effects {
            commit: None,
            clear: None,
        },
        ime_core::Effect::Commit { text } => Effects {
            commit: Some(text.clone()),
            clear: None,
        },
        ime_core::Effect::CommitAndClear { text } => Effects {
            commit: Some(text.clone()),
            clear: Some(true),
        },
        ime_core::Effect::Clear => Effects {
            commit: None,
            clear: Some(true),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_event_names() {
        let cases = [
            ("letter", Some("B".to_string()), InputEvent::Letter('b')),
            ("backspace", None, InputEvent::Backspace),
            ("delete", None, InputEvent::Delete),
            ("left", None, InputEvent::Left),
            ("right", None, InputEvent::Right),
            ("pageup", None, InputEvent::PageUp),
            ("pagedown", None, InputEvent::PageDown),
            ("space", None, InputEvent::Space),
            ("enter", None, InputEvent::Enter),
            ("escape", None, InputEvent::Escape),
            ("digit", Some("3".to_string()), InputEvent::Digit(3)),
            (
                "select",
                Some("7".to_string()),
                InputEvent::SelectCandidate(7),
            ),
        ];
        for (name, value, expected) in cases {
            let event = parse_event(&KeyRequest {
                event: name.to_string(),
                value,
            })
            .unwrap();
            assert_eq!(event, expected);
        }
    }

    #[test]
    fn rejects_unknown_events_and_missing_values() {
        assert!(parse_event(&KeyRequest {
            event: "explode".into(),
            value: None
        })
        .is_err());
        assert!(parse_event(&KeyRequest {
            event: "letter".into(),
            value: None
        })
        .is_err());
        assert!(parse_event(&KeyRequest {
            event: "digit".into(),
            value: Some("x".into())
        })
        .is_err());
    }

    #[test]
    fn effects_serialize_only_present_fields() {
        let commit = effects_from(&ime_core::Effect::Commit {
            text: "不如".into(),
        });
        let value = serde_json::to_value(&commit).unwrap();
        assert_eq!(value["commit"], "不如");
        assert!(value.get("clear").is_none());

        let clear = effects_from(&ime_core::Effect::Clear);
        assert_eq!(serde_json::to_value(&clear).unwrap()["clear"], true);
    }
}

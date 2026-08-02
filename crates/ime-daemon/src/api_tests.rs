//! End-to-end HTTP and WebSocket tests against the real router.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio_tungstenite::WebSocketStream;
use tower::ServiceExt;

use crate::api::{
    router, CommitAckResponse, DeleteSessionResponse, Effects, KeyResponse, SessionResponse,
};
use crate::session::StateSnapshot;
use crate::test_util::{model_response, prediction_response, test_store};

fn request(method: &str, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn status_of(app: &Router, method: &str, path: &str, body: serde_json::Value) -> StatusCode {
    app.clone()
        .oneshot(request(method, path, body))
        .await
        .unwrap()
        .status()
}

async fn json_response<T: DeserializeOwned>(
    app: &Router,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, T) {
    let response = app
        .clone()
        .oneshot(request(method, path, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let parsed = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "{method} {path} -> {status}: invalid body {:?}: {error}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, parsed)
}

fn texts(snapshot: &StateSnapshot) -> Vec<&str> {
    snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect()
}

#[tokio::test]
async fn http_key_flow_and_session_isolation() {
    let (store, driver) = test_store();
    let app = router(store);

    let (status, created): (StatusCode, SessionResponse) =
        json_response(&app, "POST", "/v1/sessions", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let session_id = created.session_id;

    let (_, second): (StatusCode, SessionResponse) =
        json_response(&app, "POST", "/v1/sessions", json!({})).await;

    let (status, keyed): (StatusCode, KeyResponse) = json_response(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/key"),
        json!({"event": "letter", "value": "b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(keyed.state.candidates.is_empty());
    assert!(!keyed.state.model_pending);

    let (_, keyed): (StatusCode, KeyResponse) = json_response(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/key"),
        json!({"event": "letter", "value": "u"}),
    )
    .await;
    assert_eq!(keyed.state.composition.input, "bu");
    assert!(keyed.state.model_pending);
    assert_eq!(texts(&keyed.state), vec!["不"]);

    let (_, second_state): (StatusCode, SessionResponse) = json_response(
        &app,
        "GET",
        &format!("/v1/sessions/{}/state", second.session_id),
        json!({}),
    )
    .await;
    assert!(second_state.state.composition.input.is_empty());

    driver.respond_candidates(model_response(&["d:不"]));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let state = loop {
        let (_, current): (StatusCode, SessionResponse) = json_response(
            &app,
            "GET",
            &format!("/v1/sessions/{session_id}/state"),
            json!({}),
        )
        .await;
        if !current.state.model_pending {
            break current.state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for async rerank"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(texts(&state), vec!["不"]);

    let (status, keyed): (StatusCode, KeyResponse) = json_response(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/key"),
        json!({"event": "space"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        keyed.effects,
        Effects {
            commit: Some("不".to_string()),
            clear: Some(true),
        }
    );
    assert!(keyed.state.composition.input.is_empty());

    let (status, ack): (StatusCode, CommitAckResponse) = json_response(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/commit-ack"),
        json!({"text": "不"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack.revision, 1);

    assert_eq!(
        status_of(&app, "GET", "/v1/sessions/does-not-exist/state", json!({})).await,
        StatusCode::NOT_FOUND
    );

    let (status, deleted): (StatusCode, DeleteSessionResponse) = json_response(
        &app,
        "DELETE",
        &format!("/v1/sessions/{session_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(deleted.model_reset);
    assert!(!deleted.reset_retry_scheduled);
    assert_eq!(driver.reset_requests().await.len(), 1);
    assert_eq!(
        status_of(
            &app,
            "GET",
            &format!("/v1/sessions/{session_id}/state"),
            json!({})
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn deleting_session_removes_local_state_when_model_reset_fails() {
    let (store, driver) = test_store();
    let app = router(store);
    let (_, created): (StatusCode, SessionResponse) =
        json_response(&app, "POST", "/v1/sessions", json!({})).await;
    driver.set_reset_failure(true);

    let (status, deleted): (StatusCode, DeleteSessionResponse) = json_response(
        &app,
        "DELETE",
        &format!("/v1/sessions/{}", created.session_id),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(deleted.ok);
    assert!(!deleted.model_reset);
    assert!(deleted.reset_retry_scheduled);
    assert_eq!(
        status_of(
            &app,
            "GET",
            &format!("/v1/sessions/{}/state", created.session_id),
            json!({}),
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

async fn next_state<S>(ws: &mut WebSocketStream<S>) -> StateSnapshot
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let message = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("ws snapshot timed out")
        .expect("ws closed")
        .expect("ws error");
    let text = message.into_text().expect("expected text frame");
    serde_json::from_str(&text).expect("invalid snapshot")
}

#[tokio::test]
async fn websocket_pushes_rerank_commit_and_predictions() {
    let (store, driver) = test_store();
    let app = router(store.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let created: SessionResponse = client
        .post(format!("http://{addr}/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created.session_id;

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/sessions/{session_id}/events"))
            .await
            .unwrap();
    let initial = next_state(&mut ws).await;
    assert!(initial.composition.input.is_empty());

    for letter in ['b', 'u'] {
        let response = client
            .post(format!("http://{addr}/v1/sessions/{session_id}/key"))
            .json(&json!({"event": "letter", "value": letter}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    driver.respond_candidates(model_response(&["d:不"]));
    let reranked = next_state(&mut ws).await;
    assert_eq!(reranked.composition.input, "bu");
    assert_eq!(texts(&reranked), vec!["不"]);
    assert!(!reranked.model_pending);

    let response = client
        .post(format!("http://{addr}/v1/sessions/{session_id}/key"))
        .json(&json!({"event": "space"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cleared = next_state(&mut ws).await;
    assert!(cleared.composition.input.is_empty());

    let response = client
        .post(format!("http://{addr}/v1/sessions/{session_id}/commit-ack"))
        .json(&json!({"text": "不"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pending = next_state(&mut ws).await;
    assert!(pending.model_pending);

    driver.respond_predictions(prediction_response("苹果"));
    let predicted = next_state(&mut ws).await;
    assert_eq!(predicted.predictions[0].text, "苹果");
    assert!(!predicted.model_pending);
}

#[tokio::test]
async fn healthz_reports_ready() {
    let (store, _driver) = test_store();
    let app = router(store);
    let (status, body): (StatusCode, serde_json::Value) =
        json_response(&app, "GET", "/healthz", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
}

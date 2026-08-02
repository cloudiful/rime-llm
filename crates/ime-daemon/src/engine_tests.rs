//! Engine orchestration tests: async rerank, stale-revision guards, and
//! commit-ack frequency + prediction flow.

use std::time::Duration;

use ime_core::InputEvent;
use tokio::sync::broadcast;

use crate::{
    engine::{handle_commit_ack, handle_key, model_worker},
    session::StateSnapshot,
    test_util::{model_response, prediction_response, test_store},
};

async fn snapshot_of(payload: String) -> StateSnapshot {
    serde_json::from_str(&payload).unwrap()
}

async fn next_snapshot(subscriber: &mut broadcast::Receiver<String>) -> StateSnapshot {
    let payload = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
        .await
        .expect("snapshot broadcast timed out")
        .expect("snapshot channel closed");
    snapshot_of(payload).await
}

fn texts(snapshot: &StateSnapshot) -> Vec<&str> {
    snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect()
}

#[tokio::test]
async fn key_flow_reranks_step_by_step() {
    let (store, driver) = test_store();
    let session = store.create().await;
    tokio::spawn(model_worker(store.clone(), session.clone()));
    let mut subscriber = session.tx.subscribe();

    let (snapshot, _) = handle_key(&store, &session, InputEvent::Letter('b'))
        .await
        .unwrap();
    assert!(
        snapshot.candidates.is_empty(),
        "single letter has no syllables"
    );
    assert!(!snapshot.model_pending);

    for (letter, expected_input, response_ids) in [
        ('u', "bu", vec!["d:不"]),
        ('r', "bur", vec!["d:不"]),
        ('u', "buru", vec!["d:不入", "d:不如"]),
    ] {
        let (snapshot, _) = handle_key(&store, &session, InputEvent::Letter(letter))
            .await
            .unwrap();
        assert_eq!(snapshot.composition.input, expected_input);
        assert!(snapshot.model_pending);
        driver.respond_candidates(model_response(&response_ids));
        let snapshot = next_snapshot(&mut subscriber).await;
        assert_eq!(snapshot.composition.input, expected_input);
        assert!(!snapshot.model_pending);
    }

    let seen = driver.seen_candidates().await;
    assert_eq!(seen.last().unwrap().input, "buru");
    assert!(seen.iter().any(|request| request.input == "bu"));

    let final_state = {
        let state = session.state.lock().await;
        let candidates = state
            .candidates
            .iter()
            .map(|candidate| candidate.text.clone())
            .collect::<Vec<_>>();
        (state.input.clone(), state.revision, candidates)
    };
    assert_eq!(final_state.0, "buru");
    assert_eq!(final_state.1, 4);
    assert_eq!(final_state.2, vec!["不入", "不如"]);
}

#[tokio::test]
async fn stale_model_response_never_overwrites_new_input() {
    let (store, driver) = test_store();
    let session = store.create().await;
    tokio::spawn(model_worker(store.clone(), session.clone()));
    let mut subscriber = session.tx.subscribe();

    {
        let mut state = session.state.lock().await;
        state.input = "buru".to_string();
        state.cursor = 4;
        state.revision = 7;
        store.refresh_candidates(&mut state).await;
        state.model_pending = true;
        let _ = session.model_req.send(state.revision);
    }
    tokio::time::sleep(Duration::from_millis(20)).await;

    let seen_before = driver.seen_candidates().await;
    assert!(
        seen_before.iter().any(|request| request.input == "buru"),
        "the staged request must reach the model before the stale response"
    );

    handle_key(&store, &session, InputEvent::Backspace)
        .await
        .unwrap();

    // The worker is still parked on the staged "buru" request; queue the
    // stale response first, then the response for the new revision.
    driver.respond_candidates(model_response(&["d:不如", "d:不入"]));
    driver.respond_candidates(model_response(&["d:不"]));

    let snapshot = next_snapshot(&mut subscriber).await;
    assert_eq!(snapshot.composition.input, "bur");
    assert_eq!(snapshot.revision, 8);
    assert_eq!(texts(&snapshot), vec!["不"]);
    assert!(!snapshot.model_pending);

    // The rerank broadcast only fires after the "bur" request was sent, so
    // the request must already be recorded here.
    let seen = driver.seen_candidates().await;
    assert!(seen.iter().any(|request| request.input == "bur"));
    assert!(
        matches!(
            subscriber.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ),
        "stale rerank must not produce a second broadcast"
    );
}

#[tokio::test]
async fn removing_session_stops_inflight_model_worker() {
    let (store, driver) = test_store();
    let session = store.create().await;
    let worker = tokio::spawn(model_worker(store.clone(), session.clone()));

    handle_key(&store, &session, InputEvent::Letter('b'))
        .await
        .unwrap();
    handle_key(&store, &session, InputEvent::Letter('u'))
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while driver.seen_candidates().await.is_empty() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    assert!(store.remove(&session.id).await);
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .expect("session worker did not stop after removal")
        .expect("session worker panicked");
}

#[tokio::test]
async fn commit_ack_records_frequency_and_pushes_predictions() {
    let (store, driver) = test_store();
    let session = store.create().await;
    tokio::spawn(model_worker(store.clone(), session.clone()));
    let mut subscriber = session.tx.subscribe();

    for letter in ['b', 'u', 'r', 'u'] {
        handle_key(&store, &session, InputEvent::Letter(letter))
            .await
            .unwrap();
    }
    driver.respond_candidates(model_response(&["d:不如", "d:不入"]));
    let _ = next_snapshot(&mut subscriber).await;

    let (_, effect) = handle_key(&store, &session, InputEvent::Digit(2))
        .await
        .unwrap();
    assert_eq!(
        effect,
        ime_core::Effect::CommitAndClear {
            text: "不入".to_string()
        }
    );
    let _ = next_snapshot(&mut subscriber).await;

    let revision = handle_commit_ack(store.clone(), session.clone(), "不入")
        .await
        .unwrap();
    assert_eq!(revision, 1);
    assert_eq!(store.user_freq.lock().await.count("不入"), 1);

    let _ = next_snapshot(&mut subscriber).await;
    driver.respond_predictions(prediction_response("苹果"));
    let snapshot = next_snapshot(&mut subscriber).await;
    assert_eq!(snapshot.predictions[0].text, "苹果");
    assert!(!snapshot.model_pending);
}

//! JSON wire types shared between the rime-llm model service and its
//! clients. The service and the input method daemon must serialize these
//! types identically, so they live in one crate.

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCandidate {
    pub id: String,
    pub text: String,
    pub preedit: String,
    pub consumedkeys: usize,
    pub score: f32,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRequest {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionCandidate {
    pub id: String,
    pub text: String,
    pub score: f32,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PredictionMode {
    #[default]
    Free,
    Dictionary,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResetResponse {
    pub status: String,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(request.input, "buru");
        assert_eq!(request.paths.len(), 2);
        assert_eq!(request.paths[0].text, "不如");
        assert_eq!(request.paths[0].consumedkeys, 4);
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

    #[test]
    fn prediction_mode_serializes_lowercase() {
        let value = serde_json::to_value(PredictionMode::Hybrid).unwrap();
        assert_eq!(value, "hybrid");
        let decoded: PredictionMode = serde_json::from_value("free".into()).unwrap();
        assert_eq!(decoded, PredictionMode::Free);
    }

    #[test]
    fn prediction_candidate_uses_type_wire_field() {
        let candidate = PredictionCandidate {
            id: "p0".into(),
            text: "苹果".into(),
            score: 1.0,
            kind: "llm_prediction".into(),
        };
        let value = serde_json::to_value(candidate).unwrap();
        assert_eq!(value["type"], "llm_prediction");
        assert!(value.get("kind").is_none());
    }
}

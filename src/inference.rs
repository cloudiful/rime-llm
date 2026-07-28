use anyhow::{Context, Result};
use either::Either;
use mistralrs::{
    Constraint, DType, Device, Model, NormalRequest, Request, RequestMessage, ResponseOk,
    SamplingParams, TextMessageRole, TextMessages,
};
use tokio::sync::mpsc::channel;

/// Tokenize the same chat prompt that is used for generation, without enabling
/// Qwen's thinking mode. Candidate text is appended after this prompt.
pub async fn tokenize_prompt(model: &Model, prompt: &str) -> Result<Vec<u32>> {
    let messages = TextMessages::new().add_message(TextMessageRole::User, prompt);
    model
        .tokenize(Either::Left(messages), None, false, true, Some(false))
        .await
        .context("tokenize chat prompt")
}

pub async fn tokenize_candidate(model: &Model, text: &str) -> Result<Vec<u32>> {
    model
        .tokenize(
            Either::Right(text.to_string()),
            None,
            false,
            false,
            Some(false),
        )
        .await
        .context("tokenize dictionary candidate")
}

/// Run one prompt or prompt-plus-candidate sequence and return logits for each
/// input position. Row `n - 1` predicts the token after input token `n - 1`.
pub async fn raw_logits_for_tokens(model: &Model, tokens: &[u32]) -> Result<Vec<Vec<f32>>> {
    if tokens.is_empty() {
        anyhow::bail!("cannot request logits for an empty token sequence");
    }

    let (tx, mut rx) = channel(1);
    let request = Request::Normal(Box::new(NormalRequest {
        messages: RequestMessage::CompletionTokens(tokens.to_vec()),
        sampling_params: SamplingParams {
            max_len: Some(0),
            ..SamplingParams::deterministic()
        },
        response: tx,
        return_logprobs: false,
        is_streaming: false,
        id: 0,
        constraint: Constraint::None,
        suffix: None,
        tools: None,
        tool_choice: None,
        logits_processors: None,
        return_raw_logits: true,
        web_search_options: None,
        enable_code_execution: false,
        enable_shell: false,
        shell_options: None,
        code_execution_permission: None,
        code_execution_approval_notifier: None,
        agent_permission: None,
        agent_approval_handler: None,
        agent_approval_notifier: None,
        max_tool_rounds: None,
        tool_dispatch_url: None,
        model_id: None,
        adapter: None,
        truncate_sequence: false,
        session_id: None,
        files: None,
        input_files: Vec::new(),
    }));
    model.inner().get_sender(None)?.send(request).await?;

    let response = rx
        .recv()
        .await
        .context("raw logits response channel closed")?
        .as_result()?;
    let ResponseOk::Raw {
        logits_chunks,
        tokens: returned_tokens,
    } = response
    else {
        anyhow::bail!("model returned a non-raw response");
    };
    if returned_tokens != tokens {
        anyhow::bail!("model returned a token sequence different from the request");
    }
    let tensor = logits_chunks
        .into_iter()
        .next()
        .context("model returned no logits")?
        .to_device(&Device::Cpu)?
        .to_dtype(DType::F32)?;
    match tensor.dims() {
        [_rows, _vocab] => tensor.to_vec2::<f32>().map_err(Into::into),
        [_vocab] => Ok(vec![tensor.to_vec1::<f32>()?]),
        dimensions => anyhow::bail!("unexpected logits shape: {dimensions:?}"),
    }
}

pub fn append_candidate(prompt_tokens: &[u32], candidate_tokens: &[u32]) -> Vec<u32> {
    let mut tokens = Vec::with_capacity(prompt_tokens.len() + candidate_tokens.len());
    tokens.extend_from_slice(prompt_tokens);
    tokens.extend_from_slice(candidate_tokens);
    tokens
}

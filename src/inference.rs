use std::num::NonZeroU32;

use anyhow::{Context, Result};
use encoding_rs::UTF_8;
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel},
    sampling::LlamaSampler,
    token::LlamaToken,
};

pub(crate) fn tokenize(model: &LlamaModel, text: &str, add_bos: AddBos) -> Result<Vec<u32>> {
    model
        .str_to_token(text, add_bos)
        .context("tokenize text")?
        .into_iter()
        .map(|token| u32::try_from(token.0).context("llama.cpp returned a negative token id"))
        .collect()
}

pub(crate) fn logits_at_positions(
    model: &LlamaModel,
    backend: &LlamaBackend,
    context_window: usize,
    tokens: &[u32],
    positions: &[usize],
) -> Result<Vec<Vec<f32>>> {
    if tokens.is_empty() {
        anyhow::bail!("cannot request logits for an empty token sequence");
    }
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    if tokens.len() > context_window {
        anyhow::bail!(
            "token sequence has {} tokens, exceeding context window {}",
            tokens.len(),
            context_window
        );
    }
    if positions.iter().any(|position| *position >= tokens.len()) {
        anyhow::bail!("requested logit position is outside the token sequence");
    }

    let mut context = new_context(model, backend, context_window)?;
    let mut batch = LlamaBatch::new(tokens.len(), 1);
    for (position, token) in tokens.iter().enumerate() {
        batch.add(
            llama_token(*token)?,
            i32::try_from(position).context("token position exceeds llama.cpp range")?,
            &[0],
            positions.contains(&position),
        )?;
    }
    context.decode(&mut batch).context("decode logits batch")?;

    positions
        .iter()
        .map(|position| {
            Ok(context
                .get_logits_ith(i32::try_from(*position).context("logit index overflow")?)
                .to_vec())
        })
        .collect()
}

pub(crate) fn generate(
    model: &LlamaModel,
    backend: &LlamaBackend,
    context_window: usize,
    prompt_tokens: &[u32],
    max_tokens: usize,
) -> Result<String> {
    if prompt_tokens.is_empty() {
        anyhow::bail!("cannot generate from an empty prompt");
    }
    if prompt_tokens.len() >= context_window {
        anyhow::bail!("prompt leaves no room for generated tokens");
    }
    if max_tokens == 0 {
        return Ok(String::new());
    }

    let mut context = new_context(model, backend, context_window)?;
    let mut prompt_batch = LlamaBatch::new(prompt_tokens.len(), 1);
    prompt_batch.add_sequence(
        &prompt_tokens
            .iter()
            .copied()
            .map(llama_token)
            .collect::<Result<Vec<_>>>()?,
        0,
        false,
    )?;
    context
        .decode(&mut prompt_batch)
        .context("decode generation prompt")?;

    let mut sampler = LlamaSampler::greedy();
    let mut next = sampler.sample(
        &context,
        i32::try_from(prompt_tokens.len() - 1).context("prompt index exceeds llama.cpp range")?,
    );
    sampler.accept(next);

    let mut decoder = UTF_8.new_decoder();
    let mut output = String::new();
    for generated_index in 0..max_tokens {
        if model.is_eog_token(next) {
            break;
        }
        output.push_str(
            &model
                .token_to_piece(next, &mut decoder, false, None)
                .context("decode generated token")?,
        );

        if generated_index + 1 == max_tokens
            || prompt_tokens.len() + generated_index + 1 >= context_window
        {
            break;
        }

        let mut batch = LlamaBatch::new(1, 1);
        batch.add(
            next,
            i32::try_from(prompt_tokens.len() + generated_index)
                .context("generation position exceeds llama.cpp range")?,
            &[0],
            true,
        )?;
        context
            .decode(&mut batch)
            .context("decode generated token")?;
        next = sampler.sample(&context, 0);
        sampler.accept(next);
    }

    Ok(output)
}

fn new_context<'a>(
    model: &'a LlamaModel,
    backend: &LlamaBackend,
    context_window: usize,
) -> Result<llama_cpp_2::context::LlamaContext<'a>> {
    let context_window = u32::try_from(context_window).context("context window exceeds u32")?;
    let context_window = NonZeroU32::new(context_window).context("context window is zero")?;
    let n_ubatch = context_window.get().min(512);
    let params = LlamaContextParams::default()
        .with_n_ctx(Some(context_window))
        .with_n_batch(context_window.get())
        .with_n_ubatch(n_ubatch);
    model
        .new_context(backend, params)
        .context("create llama.cpp context")
}

fn llama_token(token: u32) -> Result<LlamaToken> {
    Ok(LlamaToken::new(
        i32::try_from(token).context("token id exceeds llama.cpp range")?,
    ))
}

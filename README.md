# rime-llm

[简体中文](README.zh-CN.md)

`rime-llm` is a self-contained pinyin input method for macOS with a local
LLM service. Dictionary candidates appear immediately as you type; the local
model (Qwen3-0.6B via llama.cpp) reranks them asynchronously and suggests the
next word after a commit. It does not depend on Squirrel or librime, and the
input method keeps working with dictionary candidates when the model service
is unavailable.

## Features

- Pinyin input with instant dictionary candidates and async model reranking.
- Next-word prediction after Chinese text is committed.
- Local model service (Metal on Apple Silicon, CPU on Windows) on `127.0.0.1`.
- Bundled rime-ice dictionary; user frequency is saved locally.

## Install

1. Install `RimeLLMInputMethod.app` from the release archive into
   `~/Library/Input Methods`, then enable **Rime LLM** in
   System Settings → Keyboard → Text Input. The input method starts its own
   `ime-daemon` process automatically.
2. Start the model service (below) for candidate reranking and prediction.
   Without it, the input method still offers dictionary candidates.

## Deploy the model service

Create a local configuration:

```bash
cp config.example.toml config.toml
```

On macOS Apple Silicon:

```bash
cargo run --features metal
```

On Windows or CPU-only systems:

```bash
cargo run --no-default-features
```

The Rust build requires CMake, a C++ compiler, and clang/libclang because the
llama.cpp bindings are generated during compilation. On macOS Apple Silicon,
the `metal` feature enables GPU offload; set `device = "cpu"` in
`config.toml` when Metal is unavailable. Windows releases use the CPU
backend. The first start downloads the model into the Hugging Face cache;
set `HF_ENDPOINT` to use a mirror. The default service address is
`127.0.0.1:32123`.

## Build the input method

```bash
scripts/build-macos-ime.sh
```

The script builds `ime-daemon`, compiles `RimeLLMInputMethod.app` with
`xcodebuild`, embeds the daemon and dictionary, and ad-hoc signs the bundle.
See [`docs/ime-daemon.md`](docs/ime-daemon.md) for daemon configuration and
[`docs/ime-protocol.md`](docs/ime-protocol.md) for the input method protocol.

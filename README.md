# rime-llm

[Chinese](README.zh-CN.md)

`rime-llm` is an experimental local Rime schema backed by a Candle-based `mistral.rs` runtime. It loads a Qwen3 GGUF model, scores candidates constrained by the local Rime dictionaries, and keeps committed context in memory only.

Build and run it from this directory:

```bash
cargo build --release
cp config.example.toml config.toml
./target/release/rime-llm
```

Metal is optional. The default build uses CPU and works without Xcode's Metal Toolchain. After installing that toolchain, build with `cargo build --release --features metal` for Metal acceleration; the runtime still falls back to CPU if Metal initialization fails.

On first startup the service downloads `unsloth/Qwen3-0.6B-GGUF/Qwen3-0.6B-IQ4_XS.gguf` into `models/`. Set `device = "cpu"` when Metal is unavailable; the default is Metal with runtime CPU fallback. The model file is ignored by Git.

After rebuilding the Rime configuration, select `雾凇拼音（本地模型）` (`rime_ice_llm`) in Squirrel. The service provides `GET /healthz`, `POST /candidates`, `POST /commit`, `POST /reset`, and `GET /stats`. When it is unavailable, the experimental schema falls back to its ordinary Rime translator; the normal `rime_ice` schema is unchanged.

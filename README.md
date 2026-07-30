# rime-llm

[Chinese](README.zh-CN.md)

`rime-llm` is an experimental local Rime schema backed by a Candle-based `mistral.rs` runtime. It loads a Qwen3 GGUF model, scores candidates constrained by a local Rime dictionary, and keeps committed context in memory only.

Build and run it from this directory:

```bash
cargo build --release
cp config.example.toml config.toml
./target/release/rime-llm
```

Metal is optional. The default build uses CPU and works without Xcode's Metal Toolchain. After installing that toolchain, build with `cargo build --release --features metal` for Metal acceleration; the runtime still falls back to CPU if Metal initialization fails.

On first startup the service downloads `unsloth/Qwen3-0.6B-GGUF/Qwen3-0.6B-Q4_K_M.gguf` into `models/`. Set `device = "cpu"` when Metal is unavailable; the default is Metal with runtime CPU fallback. The model file is ignored by Git.

The repository includes a snapshot of the mature [rime-ice](https://github.com/iDvel/rime-ice) Chinese dictionary under `data/rime-ice/`. It is used by default when the service is run from this project. Set `dictionary_root` in `config.toml`, or `RIME_LLM_DICTIONARY_ROOT`, to use another Rime dictionary root. A custom root should contain `rime_ice.dict.yaml`; its enabled `import_tables` are loaded, while a root without that manifest falls back to `cn_dicts/*.yaml`.

The application code is Apache-2.0. The bundled dictionary is a separate GPL-3.0-only data asset; its license, source commit, credits, and update procedure are documented in [`data/rime-ice/SOURCE.md`](data/rime-ice/SOURCE.md). Keep both license notices when distributing the project. The dictionary can be updated independently with `scripts/update-rime-ice.sh`; the script downloads dictionary files only, not the rest of the upstream repository.

After rebuilding the Rime configuration, select `雾凇拼音（本地模型）` (`rime_ice_llm`) in Squirrel. The service provides `GET /healthz`, `POST /candidates`, `POST /commit`, `POST /reset`, and `GET /stats`. When it is unavailable, the experimental schema falls back to its ordinary Rime translator; the normal `rime_ice` schema is unchanged.

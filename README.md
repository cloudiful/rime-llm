# rime-llm

[Chinese](README.zh-CN.md)

`rime-llm` is a local candidate-reranking and next-word prediction service for
Rime. It uses Qwen3-0.6B-Q4_K_M through `mistral.rs` and keeps committed context
in memory. The model-ranked result is shown in the active candidate menu when
the service responds; ordinary Rime candidates are used as the fallback.

## Run the service

```bash
cp config.example.toml config.toml
cargo run --features metal
```

The first start downloads the model through `hf-hub` into the Hugging Face
global cache (`~/.cache/huggingface`). Set `HF_ENDPOINT` to use a mirror, or
`model_dir` to override the cache location. Use `device = "cpu"` when Metal is
unavailable. The service listens on `127.0.0.1:32123` by default.

## Use with Squirrel

The experimental `rime_ice_llm` schema is included in this repository. Plum
can install the schema with the root `recipe.yaml`; it does not modify
`default.custom.yaml` or other personal configuration.

The native plugin is currently macOS Apple Silicon only and must be built
against the librime 1.16 dylib shipped by Squirrel. Follow
[`docs/native-plugin.md`](docs/native-plugin.md) to build and copy
`librime-llm-predict.dylib` into Squirrel's bundled `rime-plugins` directory.
Then redeploy Rime and select `雾凇拼音（本地模型）` (`rime_ice_llm`).

Tagged GitHub releases provide a macOS arm64 archive containing the service and
native plugin, plus a Windows x64 CPU service archive. The model is downloaded
on first start and is not included in release archives.

The old Lua translator remains in the Rime configuration as a fallback, but
the default schema no longer loads it. When the service or plugin is
unavailable, ordinary Rime input continues to work.

## Prediction behavior

While entering ordinary pinyin, the native translator requests `/candidates`
for the current input and waits up to 1500 ms so Squirrel can display the
model-ranked menu in the same key update. If the service is unavailable or
exceeds the wait budget, ordinary Rime dictionary candidates are used. After a
Chinese commit, the worker waits 200 ms, sends one `/predict` request, and
displays up to five candidates in the background.

Modes are `free` (default), `dictionary`, and `hybrid`. Configure them in the
`prediction` block of the schema or in
`rime_ice_llm.custom.yaml.example`. `Tab`, `Enter`, and `1-9` accept a
prediction. `Space` always commits a literal space, and `Esc` closes the
prediction. Set `prediction.trigger` to prefetch without automatic popup.

The service exposes `GET /healthz`, `POST /candidates`, `POST /predict`,
`POST /commit`, `POST /reset`, and `GET /stats`.

The bundled dictionary is a snapshot of [rime-ice](https://github.com/iDvel/rime-ice)
and is separately licensed GPL-3.0-only. Keep
[`data/rime-ice/LICENSE`](data/rime-ice/LICENSE) and
[`data/rime-ice/SOURCE.md`](data/rime-ice/SOURCE.md) when redistributing it.
Update dictionary files only with `scripts/update-rime-ice.sh`. The
application code is Apache-2.0.

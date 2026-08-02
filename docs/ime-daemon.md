# ime-daemon

`ime-daemon` is the local composition engine of the macOS input method. It is
a small Rust binary (`crates/ime-daemon`) that exposes the HTTP/WebSocket
protocol described in [`docs/ime-protocol.md`](docs/ime-protocol.md).

## Configuration

Environment variables (defaults shown):

| Variable | Default |
| --- | --- |
| `RIME_LLM_DAEMON_BIND` | `127.0.0.1:32124` |
| `RIME_LLM_SERVICE_URL` | `http://127.0.0.1:32123` |
| `RIME_LLM_DICTIONARY_ROOT` | `data/rime-ice` |
| `RIME_LLM_USER_FREQ` | `~/Library/Application Support/RimeLLM/user_freq.txt` |
| `RIME_LLM_MAX_CANDIDATES` | `16` |
| `RIME_LLM_MODEL_TIMEOUT_MS` | `15000` |
| `RIME_LLM_DAEMON_LOG` | `info` |

The daemon loads the dictionary at startup and caches it in memory. The model
service is optional at runtime: candidate reranks and predictions fail
softly, and the daemon keeps serving dictionary candidates.

Closing a local session removes its composition state immediately and sends an
idempotent reset request for the same session to the model service. Reset
failures are retried briefly in the background and do not block input-method
shutdown.

## Running manually

```bash
cargo run -p ime-daemon
```

The input method launches the bundled `ime-daemon` (from
`Contents/Resources/ime-daemon`) automatically when the app starts, with
`RIME_LLM_DICTIONARY_ROOT` pointing at the embedded
`Contents/Resources/data/rime-ice`. Its logs go to
`~/Library/Logs/RimeLLM/ime-daemon.log`; set `RIME_LLM_DAEMON_LOG=debug` for
more detail. The default log level does not print per-key input content.

## Workspace layout

- `crates/model-protocol` — wire types shared with the model service.
- `crates/pinyin-dict` — Rime dictionary loading, syllable indexing, user
  frequency persistence.
- `crates/ime-core` — pinyin segmentation, candidate lattice, input state
  machine.
- `crates/ime-daemon` — sessions, HTTP/WebSocket API, model client.

## Building the input method

```bash
scripts/build-macos-ime.sh
```

Steps performed:

1. `cargo build --release -p ime-daemon` (skip with `IME_DAEMON_BIN`).
2. `xcodebuild` compiles `RimeLLMInputMethod.app` (arm64 by default; override
   with `ARCHS`).
3. The daemon binary and `data/rime-ice` are embedded into
   `Contents/Resources`.
4. The bundle is ad-hoc signed (`codesign --force --deep --sign -`).

KeyMapper logic is verified separately:

```bash
scripts/test-keymapper.sh
```

## Release packaging

`scripts/package-release.sh` (macOS) produces a tarball with:

- `rime-llm` — the model service (built with `--features metal`).
- `ime-daemon` — the local composition daemon.
- `RimeLLMInputMethod.app` — the input method (when built).
- `data/rime-ice` and `config.example.toml`.

`scripts/package-release.ps1` (Windows) ships only the model service, matching
the v1 scope (no Windows input method frontend).

## Manual end-to-end check

```bash
cargo run --features metal            # terminal 1: model service
cargo run -p ime-daemon               # terminal 2: daemon
curl -X POST http://127.0.0.1:32124/v1/sessions -d '{}' \
  -H 'content-type: application/json'
```

Then send key events to `/v1/sessions/{id}/key` and watch the WebSocket at
`/v1/sessions/{id}/events` for rerank and prediction snapshots.

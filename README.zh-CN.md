# rime-llm

[English](README.md)

`rime-llm` 是一款自带本地模型服务的 macOS 拼音输入法。输入拼音时词典
候选即时出现，本地模型（llama.cpp 运行 Qwen3-0.6B）异步重排候选，并在
上屏后给出下一词预测。它不依赖 Squirrel 与 librime；模型服务不可用时，
输入法仍提供词典候选。

## 功能

- 拼音输入，词典候选即时出现，模型异步重排后自动刷新。
- 中文上屏后提供下一词预测。
- 本地模型服务（Apple Silicon 使用 Metal，Windows 使用 CPU），监听 `127.0.0.1`。
- 内置雾凇词库；用户词频保存在本地。

## 安装

1. 将发布包中的 `RimeLLMInputMethod.app` 安装到 `~/Library/Input Methods`，
   然后在 系统设置 → 键盘 → 文本输入 中启用 **Rime LLM**。输入法会自动
   启动同包内的 `ime-daemon` 进程。
2. 启动模型服务（见下）以获得候选重排与预测；未启动时输入法仍可使用
   词典候选。

## 部署模型服务

创建本地配置并启动服务：

```bash
cp config.example.toml config.toml
```

macOS Apple Silicon：

```bash
cargo run --features metal
```

Windows 或仅使用 CPU：

```bash
cargo run --no-default-features
```

Rust 构建需要 CMake、C++ 编译器和 clang/libclang，因为 llama.cpp 绑定会在构建时生成。
macOS Apple Silicon 使用 `metal` feature 启用 GPU offload；仅使用 CPU 或 Metal 不可用时，
在配置中设置 `device = "cpu"`。Windows 发布包使用 CPU 后端。

首次启动会把模型下载到 Hugging Face 缓存。需要使用镜像时设置 `HF_ENDPOINT`；
服务默认监听 `127.0.0.1:32123`。llama.cpp 和 ggml 的诊断日志默认关闭；
排查模型或设备初始化问题时，可设置 `RIME_LLM_LLAMA_LOG=1`，将其转发到
服务日志。

## 构建输入法

```bash
scripts/build-macos-ime.sh
```

脚本会构建 `ime-daemon`、用 `xcodebuild` 编译 `RimeLLMInputMethod.app`、
嵌入守护进程与词典，并做 ad-hoc 签名。守护进程配置见
[`docs/ime-daemon.md`](docs/ime-daemon.md)，输入法协议见
[`docs/ime-protocol.md`](docs/ime-protocol.md)。

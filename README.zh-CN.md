# rime-llm

[English](README.md)

`rime-llm` 是一个实验性的本地 Rime 方案，使用 Candle 生态的 `mistral.rs` 运行 Qwen3 GGUF 模型。它只在本地雾凇词库候选中排序，提交上下文只保存在进程内存中。

在本目录构建并运行：

```bash
cargo build --release
cp config.example.toml config.toml
./target/release/rime-llm
```

Metal 是可选功能。默认构建使用 CPU，不依赖 Xcode Metal Toolchain。安装该组件后，可使用 `cargo build --release --features metal` 启用 Metal 加速；运行时初始化失败仍会回退 CPU。

首次启动会从 Hugging Face 下载 `unsloth/Qwen3-0.6B-GGUF/Qwen3-0.6B-IQ4_XS.gguf` 到 `models/`。默认使用 Metal，运行时初始化失败会回退 CPU；也可以将 `device = "cpu"` 写入配置。模型文件已加入 Git 忽略规则。

重新部署 Rime 配置后，在鼠须管中选择「雾凇拼音（本地模型）」（`rime_ice_llm`）。服务提供 `GET /healthz`、`POST /candidates`、`POST /commit`、`POST /reset` 和 `GET /stats`。服务不可用时实验方案自动回退普通 Rime 翻译器；普通 `rime_ice` 方案不受影响。

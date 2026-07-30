# rime-llm

[English](README.md)

`rime-llm` 是一个实验性的本地 Rime 方案，使用 Candle 生态的 `mistral.rs` 运行 Qwen3 GGUF 模型。它只在本地 Rime 词典候选中排序，提交上下文只保存在进程内存中。

在本目录构建并运行：

```bash
cargo build --release
cp config.example.toml config.toml
./target/release/rime-llm
```

Metal 是可选功能。默认构建使用 CPU，不依赖 Xcode Metal Toolchain。安装该组件后，可使用 `cargo build --release --features metal` 启用 Metal 加速；运行时初始化失败仍会回退 CPU。

首次启动会从 Hugging Face 下载 `unsloth/Qwen3-0.6B-GGUF/Qwen3-0.6B-Q4_K_M.gguf` 到 `models/`。默认使用 Metal，运行时初始化失败会回退 CPU；也可以将 `device = "cpu"` 写入配置。模型文件已加入 Git 忽略规则。

项目自带一份成熟的[雾凇词库](https://github.com/iDvel/rime-ice)快照，位于 `data/rime-ice/`，从项目目录启动时默认使用。需要换词典时，在 `config.toml` 设置 `dictionary_root`，或设置环境变量 `RIME_LLM_DICTIONARY_ROOT`。自定义目录应包含 `rime_ice.dict.yaml`；程序会按其中启用的 `import_tables` 加载词表。没有这个清单时，为兼容旧用法会加载 `cn_dicts/*.yaml`。

项目代码采用 Apache-2.0；内置词典是独立的 GPL-3.0-only 数据资产，不会因为随项目分发就变成 Apache-2.0。许可证、来源 commit、致谢、文件哈希和更新方法见 [`data/rime-ice/SOURCE.md`](data/rime-ice/SOURCE.md)。运行 `scripts/update-rime-ice.sh <commit>` 可以只更新词典文件，不会拉取上游仓库的其他内容；再次分发时请保留这两套许可证说明。

重新部署 Rime 配置后，在鼠须管中选择「雾凇拼音（本地模型）」（`rime_ice_llm`）。服务提供 `GET /healthz`、`POST /candidates`、`POST /commit`、`POST /reset` 和 `GET /stats`。服务不可用时实验方案自动回退普通 Rime 翻译器；普通 `rime_ice` 方案不受影响。

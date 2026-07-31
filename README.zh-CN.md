# rime-llm

[English](README.md)

`rime-llm` 是 Rime 的本地下一词预测服务。它使用 `mistral.rs` 运行
Qwen3-0.6B-Q4_K_M，只在内存中保存已上屏上下文，普通 Rime 词典候选仍会立即显示。

## 启动服务

```bash
cp config.example.toml config.toml
cargo run --features metal
```

首次启动会通过 `hf-hub` 把模型下载到 Hugging Face 全局缓存
（`~/.cache/huggingface`）。设置 `HF_ENDPOINT` 可切换镜像，设置 `model_dir`
可覆盖缓存位置。没有 Metal 时将 `device = "cpu"` 写入配置。
服务默认监听 `127.0.0.1:32123`。

## 在鼠须管中使用

仓库包含实验方案 `rime_ice_llm`。根目录的 `recipe.yaml` 可供 Plum 安装独立
schema，不会修改 `default.custom.yaml` 或其他个人配置。

原生插件首版只支持 macOS Apple Silicon，并且必须针对鼠须管自带的 librime 1.16
动态库构建。请按 [`docs/native-plugin.md`](docs/native-plugin.md) 构建并复制
`librime-llm-predict.dylib` 到鼠须管自带的 `rime-plugins` 目录，然后重新部署 Rime，
选择「雾凇拼音（本地模型）」(`rime_ice_llm`)。

原 Lua 翻译器仍保留在 Rime 配置中作为回退，但默认方案已经不再加载它。服务或插件
不可用时，普通 Rime 输入仍然可以使用。

## 预测行为

普通拼音输入时，native worker 会在后台请求 `/candidates`，普通 Rime 词典候选仍然立即显示。
提交中文后，worker 等待 200ms，发送一次 `/predict` 请求并显示最多五个候选。
worker 不阻塞 Rime 按键路径，同时最多运行一个模型请求，只保留一个最新待处理请求。

模式有 `free`（默认）、`dictionary` 和 `hybrid`，可在 schema 的 `prediction` 配置块或
`rime_ice_llm.custom.yaml.example` 中调整。`Tab`、`Enter` 和 `1-9` 接受预测词；`Space`
永远上屏普通空格；`Esc` 关闭预测。设置 `prediction.trigger` 可只预取而不自动弹出候选框。

服务提供 `GET /healthz`、`POST /candidates`、`POST /predict`、`POST /commit`、
`POST /reset` 和 `GET /stats`。

内置词典是 [雾凇词库](https://github.com/iDvel/rime-ice) 的快照，作为独立的
GPL-3.0-only 数据资产分发。再次分发时请保留
[`data/rime-ice/LICENSE`](data/rime-ice/LICENSE) 和
[`data/rime-ice/SOURCE.md`](data/rime-ice/SOURCE.md)，只使用
`scripts/update-rime-ice.sh` 更新词典文件。项目代码采用 Apache-2.0。

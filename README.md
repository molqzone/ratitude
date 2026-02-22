# Ratitude

High-performance binary telemetry stack for embedded systems.

## 核心能力

- C/C++ `@rat` 声明驱动的数据定义
- COBS 帧传输与动态解码
- 单入口 `rttd` 交互式 daemon
- 运行时命令台控制 source/输出
- JSONL 与 Foxglove 输出

## 架构

- `rat-config`: 配置模型与 TOML 读写
- `rat-sync`: 声明扫描与生成文件
- `rat-protocol`: COBS + packet 解析
- `rat-core`: transport/hub/logger
- `rat-bridge-foxglove`: Foxglove 输出桥接
- `rttd`: 运行时 daemon

## 快速启动

```bash
cargo run -p rttd -- --config firmware/example/stm32f4_rtt/rat.toml
```

启动后可用命令：

- `$help`
- `$status`
- `$source list`
- `$source use <index>`
- `$foxglove on|off`
- `$jsonl on|off [path]`
- `/packet/<struct>/<field>`

说明：

- 命令台是主配置入口之一。
- `$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。

## Mock 联调

```bash
./tools/run_mock_foxglove.sh
```

## 文档入口

- `docs/README.md`
- `docs/quickstart.md`
- `docs/workflow.md`
- `docs/config-files.md`

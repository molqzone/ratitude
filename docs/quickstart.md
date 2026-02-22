# 快速开始（5 分钟）

## 1. 准备

- 已有 `@rat` 声明的 C/C++ 源码
- 已存在可连接的 RTT 端点（例如由 OpenOCD/J-Link 预先提供）
- `rat.toml` 使用 v0.2 结构

## 2. 启动 daemon

```bash
cargo run -p rttd -- --config firmware/example/stm32f4_rtt/rat.toml
```

启动后先执行：

```text
$help
$status
$source list
```

## 3. 常用运行命令

```text
$foxglove on
$jsonl on out.jsonl
```

这些命令中，`$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。
注意：`$source list` / `$source use` 执行前会实时刷新候选，索引可能随可达性变化。

停止：

```text
$quit
```

## 4. 无硬件 mock 联调

```bash
./tools/run_mock_foxglove.sh
```

脚本会启动 mock RTT 字节流，然后启动 daemon。Foxglove 输出由 `rat.toml` 中 `[rttd.outputs.foxglove]` 控制。

## 5. 重点校验

- 看到 source 候选列表并可切换
- 启动后在 `schema_timeout` 内进入 schema ready
- `$foxglove on` 后可连接到 `ws://127.0.0.1:8765`
- schema hash 不一致时会 fail-fast

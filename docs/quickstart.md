# 快速开始（5 分钟）

## 1. 准备

- 已有 `@rat` 声明的 C/C++ 源码
- 可用 RTT backend（OpenOCD/J-Link）
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
$sync
$foxglove on
$jsonl on out.jsonl
```

这些命令中，`$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。

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
- `$sync` 能完成同步并输出结果
- `$foxglove on` 后可连接到 `ws://127.0.0.1:8765`
- 指纹不一致时会 fail-fast

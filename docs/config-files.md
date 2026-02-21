# 配置与生成文件说明（v0.2）

## 文件分工

## `rat.toml`（手工维护 + 命令台持久化）

用途：运行时 wiring 与扫描范围配置。

核心区块：

- `[project]`：源码扫描范围
- `[artifacts]`：产物路径
- `[generation]`：生成文件位置
- `[rttd.source]`：source 扫描与目标端点选择
- `[rttd.behavior]`：自动同步与 runtime 行为
- `[rttd.outputs.jsonl]`：JSONL 输出
- `[rttd.outputs.foxglove]`：Foxglove 输出

示例：

```toml
[project]
name = "stm32f4_rtt"
scan_root = "Core"
recursive = true
extensions = [".h", ".c"]

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd]
text_id = 255

[rttd.source]
auto_scan = true
scan_timeout_ms = 300
last_selected_addr = "127.0.0.1:19021"

[rttd.behavior]
auto_sync_on_start = true
auto_sync_on_reset = true
sync_debounce_ms = 500
reconnect = "1s"
buf = 256
reader_buf = 65536

[rttd.outputs.jsonl]
enabled = true
path = ""

[rttd.outputs.foxglove]
enabled = false
ws_addr = "127.0.0.1:8765"
```

说明：

- `rttd` 不再负责启动/管理 backend 进程；仅连接既有 RTT 端口。
- `last_selected_addr` 的端口号是 J-Link RTT Telnet 端口的唯一来源（`rtt_telnet_port` 已移除）。
- 命令台是主配置入口之一：`$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会写回该文件。
- 设计约束：runtime 每次重启都会重新创建 JSONL writer；当 `rttd.outputs.jsonl.enabled = true` 且配置了 `path` 时，目标文件会被清空重写（非追加）。

## `rat_gen.toml`（自动生成）

用途：主机解码声明来源。

特点：

- 不建议人工编辑
- 由内部同步逻辑更新
- `packets[*].source` 仅用于溯源，不参与签名身份

## `rat_gen.h`（自动生成）

用途：固件编译期 ID 与指纹宏。

包含：

- `RAT_GEN_FINGERPRINT`
- `RAT_GEN_PACKET_COUNT`
- `RAT_ID_<STRUCT_NAME>`

## `.rttdignore`

用途：在同步扫描时排除源码路径。

示例：

```gitignore
build/**
Drivers/**
.git/**
```

规则：

- 支持注释行（`#`）和空行
- 支持 glob
- 不支持 `!` 反选

# 配置与生成文件说明

## 文件分工

## `rat.toml`（手工维护）

用途：项目级配置与运行配置。

关键区块：

- `[project]`：源码扫描范围
- `[artifacts]`：elf/hex/bin 路径
- `[generation]`：`rat_gen.*` 生成位置与文件名
- `[rttd.server.*]`：RTT 连接参数与 backend 启动参数
- `[rttd.foxglove]`：仅保留 `ws_addr`（严格拒绝未知字段）

示例：

```toml
[project]
name = "stm32f4_rtt"
scan_root = "Core"
recursive = true
extensions = [".h", ".c"]
ignore_dirs = ["build", "Drivers", ".git"]

[artifacts]
elf = "build/Debug/stm32f4_rtt.elf"

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd.server]
addr = "127.0.0.1:19021"
reconnect = "1s"
buf = 256
reader_buf = 65536

[rttd.foxglove]
ws_addr = "127.0.0.1:8765"
```

## `rat_gen.toml`（自动生成，主机读取）

用途：保存“声明解析结果 + 分配后的 packet id + 指纹”。

特点：

- 不建议人工编辑
- 每次 `rttd sync` 可能更新
- `rttd server` / `rttd foxglove` 运行时都只使用它作为解码声明来源
- `rat_gen.toml` 缺失或 `packets=[]` 时，两种模式都会直接失败
- `packets[*].source` 仅用于审计溯源（声明来自哪个源文件），不参与 packet 身份签名与 ID 分配

`rttd.server.reader_buf` 语义：

- 作用于 RTT 传输层零分隔帧读取缓冲区（bytes）
- 必须 `> 0`
- `server` 可通过 `--reader-buf` 临时覆盖该值

## `rat_gen.h`（自动生成，固件编译使用）

用途：固件侧 packet id 与配置指纹宏。

典型内容：

- `RAT_GEN_FINGERPRINT`
- `RAT_GEN_PACKET_COUNT`
- `RAT_ID_<STRUCT_NAME>`

## Mock 专用配置（开箱联调）

新增目录：`examples/mock/`

- `examples/mock/rat.toml`
- `examples/mock/rat_gen.toml`

用途：

- 为 `tools/openocd_rtt_mock.py` 和 `rttd foxglove` 提供一套独立、可复现的本地联调配置
- 所有 mock 发包 ID 与字段结构严格来源于 `examples/mock/rat_gen.toml`

## 路径解析规则

- 相对路径按 `rat.toml` 所在目录解析
- `generation.out_dir` 也是相对 `rat.toml` 目录
- mock 场景建议直接使用 `examples/mock/rat.toml`，避免污染真实固件工程配置

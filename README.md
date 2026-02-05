# Ratitude

> **High-Performance Binary Telemetry Stack for Embedded Systems.**

Ratitude 是一个高性能的嵌入式 **Realtime Transfer 协议栈**。它面向 RTT 二进制遥测场景，提供固件端 librat 与宿主端 Go Host（`rttd`）核心管道，实现从 TCP 数据流到结构化解码与日志输出的完整闭环。

---

## 核心功能

Ratitude 利用 **C-struct 二进制流** 与 **COBS 编码** 取代传统字符串打印模式，将嵌入式调试从“文本日志”升级为“结构化数据流”。

### 主要特性

* **高性能二进制传输:** 直接传输 C 语言结构体 (Binary Struct)，相比 `printf` 格式化字符串，带宽利用率提升，且无精度丢失。
* **Go Host 核心管道:** `transport`/`protocol`/`engine`/`logger` 组成单向数据流管道，完成 TCP 接收、COBS 解码、结构体解析与广播。
* **JSONL 日志输出:** 内置 JSONL 写入器，便于对接脚本、数据分析或可视化工具。
* **广泛硬件兼容:** 基于标准 TCP 协议对接 OpenOCD、J-Link GDB Server 或 pyOCD。
* **OpenOCD RTT 兼容:** 固件端提供 SEGGER RTT 控制块，便于 RTT 端口直读 COBS 帧。

> 说明：MCP 仍在规划中，当前提供 `rttd server`。

## 快速开始

```bash
git submodule update --init --recursive
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

### 常用参数

* `--addr`：TCP 地址（默认 `127.0.0.1:19021`）
* `--log`：JSONL 输出路径（默认 stdout）
* `--text-id`：文本包 ID（默认 `0xFF`）

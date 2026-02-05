# Ratitude

> **High-Performance Binary Telemetry Stack for Embedded Systems.**

Ratitude 是一个高性能的嵌入式**Realtime Transfer 协议栈**。它旨在解决传统 RTT 调试中带宽利用率低、数据非结构化的问题，提供从固件库到宿主端可视化的一站式解决方案。

---

## 核心功能

Ratitude 利用 **C-struct 二进制流** 与 **COBS 编码** 取代了 SEGGER-RTT 传统的字符串打印模式，将嵌入式调试从“文本日志”升级为“结构化数据流”。

### 主要特性

* **高性能二进制传输:** 直接传输 C 语言结构体 (Binary Struct)，相比 `printf` 格式化字符串，带宽利用率提升 10 倍以上，且无精度丢失。
* **Go 单体宿主引擎:** 宿主端采用 **Go** 编写，单一二进制文件即可提供 TCP 连接管理、协议解码、以及基于 TUI 的**实时波形仪表盘**。
* **广泛的硬件兼容:** 基于标准 TCP 协议对接 OpenOCD、J-Link GDB Server 或 pyOCD，无需特定驱动即可支持所有主流 ARM/RISC-V 调试器。
* **开放的数据接口:** 除了内置的 TUI 显示，引擎还支持以 JSON 流或 MCP 协议转发数据，方便对接脚本、第三方分析工具或 AI 辅助开发。

## 快速开始

```bash
rttd server --raw

rttd mcp

rttd tui
```

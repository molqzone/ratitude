# rttd 文档导航（新手入口）

如果你第一次接触 Ratitude / rttd，建议按下面顺序阅读：

1. [快速开始（5 分钟）](./quickstart.md)
2. [rttd 工作流总览](./workflow.md)
3. [配置与生成文件说明](./config-files.md)
4. [@rat 声明规范](./rat-annotation-spec.md)
5. [路线图（设计目标与待实现项）](./roadmap.md)

---

## rttd 是什么

`rttd` 是 Ratitude 的主机侧运行时工具，负责：

- 从 RTT TCP 端口读取固件发送的 COBS 帧数据
- 按配置解析二进制 payload 为结构化数据
- 输出 JSONL 或转发到 Foxglove
- 基于源码声明自动生成 `rat_gen.toml` / `rat_gen.h`

---

## 你需要知道的三个文件

- `rat.toml`：你维护的项目配置（扫描范围、产物路径、运行参数）
- `rat_gen.toml`：工具生成（主机读取）
- `rat_gen.h`：工具生成（固件编译使用）

---

## 典型使用方式

```bash
# 1) 根据 C 源码中的 @rat 声明生成 rat_gen.*
cargo run -p rttd -- sync --config firmware/example/stm32f4_rtt/rat.toml

# 2) 启动后端（示例：J-Link RTT）
./tools/jlink_rtt_server.sh --device STM32F407ZG --if SWD --speed 4000 --rtt-port 19021

# 3) 启动 rttd
cargo run -p rttd -- server --config firmware/example/stm32f4_rtt/rat.toml --log out.jsonl
```


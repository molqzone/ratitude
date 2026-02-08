# 设计目标与路线图

本页把 `design/` 里的目标整理成“当前状态 + 后续计划”。

## A. 已实现

- `@rat, <type>` 新语法（支持省略 type）
- 自动 ID 分配（结构签名哈希 + 冲突线性探测）
- 自动生成 `rat_gen.toml` 与 `rat_gen.h`
- Foxglove 声明驱动发布（通道/schema 仅由 `rat_gen.toml` 决定）
- 固件 `rat_init` 发送 init magic（含配置指纹）
- rttd 接收并识别 init magic
- OpenOCD / J-Link 并行 backend 支持
- OpenOCD RTT 字节流 mock 联调链路（`tools/openocd_rtt_mock.py` + 一键脚本）

## B. 计划中

- 使用 `notify` 实现目录热监控与增量同步
- 声明生命周期状态管理（Pending / Active）
- backend 进程自动探测（openocd / pyocd / jlink）并交互选择
- artifact 自动发现（elf/hex 多候选选择）
- 缓存机制（避免重复全量扫描）

## C. 目标体验

希望最终达到：

1. 用户只写 `@rat` 声明，不关心 packet id
2. rttd 自动发现/生成/校验
3. 固件一启动，主机即可感知配置一致性
4. 新手仅通过 `rat.toml + 两条命令` 即可跑通链路

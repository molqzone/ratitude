# 设计目标与路线图

本页把 `design/` 里的目标整理成“当前状态 + 后续计划”。

## A. 已实现

- `@rat, <type>` 新语法（支持省略 type）
- 自动 ID 分配（结构签名哈希 + 冲突线性探测）
- 自动生成 `rat_gen.h`
- runtime schema 控制帧（HELLO/CHUNK/COMMIT）接入
- rttd 在 schema ready 后动态注册解码与输出
- OpenOCD / J-Link RTT 端点兼容
- OpenOCD RTT 字节流 mock 联调链路（`tools/openocd_rtt_mock.py` + 一键脚本）

## B. 计划中

- RTT 端点自动探测（不托管 backend 进程）并交互选择
- artifact 自动发现（elf/hex 多候选选择）
- 缓存机制（避免重复全量扫描）

## C. 目标体验

希望最终达到：

1. 用户只写 `@rat` 声明，不关心 packet id
2. `ratsync` 生成 `rat_gen.h` 后，rttd 自动连接并等待 runtime schema
3. 固件一启动，主机即可进入 schema ready 并开始解码
4. 新手仅通过 `rat.toml + 两条命令（ratsync / rttd）` 即可跑通链路

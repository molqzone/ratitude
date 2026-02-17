# Ratitude 全项目 Linus 风格审计报告

审计时间基线：2026-02-17T16:12:45Z（UTC）  
审计方法：基于 `AGENT.md`（Linus 三问 + 五层拆解）  
审计对象：当前工作区（含未提交改动与子模块本地改动）

## 1. 审计范围与基线

纳入范围：

1. `crates/rat-bridge-foxglove`
2. `crates/rat-config`
3. `crates/rat-core`
4. `crates/rat-protocol`
5. `crates/rat-sync`
6. `crates/rttd`
7. `tools/`
8. `firmware/example/stm32f4_rtt`（仅项目自定义代码）
9. `docs/` 与 `README.md`

排除范围：

1. 第三方代码（HAL/CMSIS/vendor）
2. 构建产物（`target/`、`build/`）
3. 纯生成物的“代码风格”审查（但检查与源声明的一致性）

基线快照（工作区）：

1. 根仓存在未提交改动：`README.md`、`crates/*`、`docs/*`
2. 子模块 `firmware` 存在本地改动（`git submodule status` 显示 `heads/master`）
3. `tests/rat/*` 被删除（当前基线中存在该变更）
4. `cargo test -q` 通过（多组测试全部通过）

规模采样：

1. crate 数量：6（`rat-bridge-foxglove`, `rat-config`, `rat-core`, `rat-protocol`, `rat-sync`, `rttd`）
2. 文件数采样：`crates` 16、`tools` 10、`docs` 6

## 2. 核心判断（结论先行）

[Core Judgment]  
✅ 值得做：主线“纯声明驱动”已经基本收敛，但还留着一个致命一致性缺口（init fingerprint 未校验）和几个会制造未来兼容债的结构问题；现在修，成本低、收益高。  

[Key Insights]  

- Data structure：当前真实单一事实源已收敛到 `rat_gen.toml`（`crates/rttd/src/main.rs:241`、`crates/rttd/src/main.rs:339`）。  
- Complexity：可删除复杂度点在于“路径参与签名导致 ID 漂移”和“无效配置项 reader_buf 继续暴露”。  
- Risk points：最大破坏风险是“主机不校验 firmware fingerprint”，会把声明漂移降级成静默丢包，而不是 fail-fast。  

[Linus-Style Solution]  

1. 先改数据结构语义：把 fingerprint 校验前置为握手硬门禁。  
2. 再删特殊分支：去掉无效入口（`reader_buf`）与路径耦合签名。  
3. 用最笨但最清晰的方式实现：server/foxglove 共用同一启动校验与同一错误语义。  
4. 保证零破坏迁移：为会影响已有工程的变更提供显式迁移步骤（同步文档与示例）。  

[Taste Score]  
🟡 So-so（主线方向正确，但仍有关键“坏味道”未清理干净）

[Fatal Issues]  

- RA-001：init magic 只记录不校验，声明漂移不能在入口被硬阻断。

[Improvement Directions]  

- “把漂移检测前移到握手期”，不要在业务期靠 unknown packet 告警兜底。  
- “签名去路径化”，让 ID 稳定性绑定结构定义本身，而不是文件位置。  
- “删死参数”，任何 CLI/TOML 暴露的字段都必须有运行时效果。  

## 3. 关键洞察

1. 你们已经完成了最关键的一步：`PacketData` 收敛为 `Text + Dynamic`，`server/foxglove` 启动都强依赖 `rat_gen.toml`，旧静态分支已大幅清除（`crates/rat-protocol/src/lib.rs:39`、`crates/rttd/src/main.rs:241`、`crates/rttd/src/main.rs:334`）。
2. 当前系统的主要风险已从“legacy 分支并存”转成“声明一致性守门不够硬”。具体就是：握手阶段知道 fingerprint，但不校验。
3. 现在最值钱的整改不是再加功能，而是继续删复杂度和语义歧义。

## 4. 五层拆解结果（全局）

第一层（数据结构）：

1. 主机解析真源基本统一为 `rat_gen.toml`，方向正确。  
2. 但签名哈希把 `source` 路径纳入结构身份，导致“位置变化影响身份”（`crates/rat-sync/src/lib.rs:699`）。

第二层（特殊分支）：

1. 未知包处理仍是“warn + drop”业务期兜底（`crates/rttd/src/main.rs:634`），但前置握手未建立强校验。  
2. `reader_buf` 参数保留但未实际参与链路（`crates/rttd/src/main.rs:272`）。

第三层（复杂度）：

1. `rat-sync` 的结构体布局靠自研近似规则推导（`crates/rat-sync/src/lib.rs:473`），理论上可跑，但对 ABI 边界敏感。  
2. `auto_sync_before_parse` 让运行命令带隐式写操作（`crates/rttd/src/main.rs:171`），提升了“命令副作用”复杂度。

第四层（破坏性）：

1. 如果结构声明和固件产物漂移，当前行为不是硬失败，而是运行期丢包。对用户表现是“看起来连上了但数据不对”。  
2. 路径参与签名会把重构（文件移动/重命名）变成潜在协议变更。

第五层（实用性）：

1. 现有复杂度中，最不划算的是“允许系统带漂移继续跑”。  
2. 真正需要的是更早失败，而不是更晚告警。

## 5. 模块级评审表（含证据）

| 模块 | 味道 | 结论 | 关键证据 |
|---|---|---|---|
| `crates/rttd` | 🟡 | 启动 fail-fast 语义已统一，但握手校验缺失且存在死参数 | `crates/rttd/src/main.rs:241`, `crates/rttd/src/main.rs:339`, `crates/rttd/src/main.rs:628`, `crates/rttd/src/main.rs:272` |
| `crates/rat-sync` | 🟡 | 声明扫描与生成链路完整，但签名耦合路径、布局推导有 ABI 风险 | `crates/rat-sync/src/lib.rs:249`, `crates/rat-sync/src/lib.rs:699`, `crates/rat-sync/src/lib.rs:473` |
| `crates/rat-protocol` | 🟢 | 数据模型已收敛到 Text+Dynamic，unknown id 明确报错 | `crates/rat-protocol/src/lib.rs:39`, `crates/rat-protocol/src/lib.rs:160` |
| `crates/rat-bridge-foxglove` | 🟡 | 声明驱动通道可用；image 为字段派生帧，需持续与文档口径一致 | `crates/rat-bridge-foxglove/src/lib.rs:488`, `crates/rat-bridge-foxglove/src/lib.rs:493` |
| `crates/rat-config` | 🟢 | 配置模型已去除 legacy `foxglove` 包定义字段，方向正确 | `crates/rat-config/src/lib.rs:267` |
| `crates/rat-core` | 🟢 | JSONL 输出仅 Text/Dynamic，数据面简化完成 | `crates/rat-core/src/logger.rs:66` |
| `firmware/example/stm32f4_rtt` | 🟢 | 示例已对齐 `@rat + RAT_ID_*` 链路 | `firmware/example/stm32f4_rtt/Core/Src/main.c:47`, `firmware/example/stm32f4_rtt/Core/Src/main.c:130` |
| `docs/` + `README.md` | 🟢 | 文档主口径已基本一致：server/foxglove 同样 fail-fast | `README.md:146`, `docs/workflow.md:50`, `docs/quickstart.md:83`, `docs/config-files.md:53` |

## 6. 问题清单与分级（P0~P3）

### 问题ID：RA-001

级别：P0 + 味道评分 🔴  
证据：`crates/rttd/src/main.rs:589`、`crates/rttd/src/main.rs:628`、`docs/workflow.md:38`  
问题描述（一句话）：init magic 含 fingerprint，但主机只记日志不校验，声明漂移无法在入口被阻断。  
为什么是坏味道：这是“把应当在握手层解决的问题，推迟到业务层告警”的典型坏设计。  
影响范围：`server`/`foxglove` 全链路，表现为静默丢包、数据不可信、排障成本高。  
Linus式修复方向：在 `spawn_frame_consumer` 前注入 expected fingerprint，首包握手必须比对，不一致立即 fail-fast 退出。  

### 问题ID：RA-002

级别：P1 + 味道评分 🔴  
证据：`crates/rat-sync/src/lib.rs:249`、`crates/rat-sync/src/lib.rs:699`、`crates/rat-sync/src/lib.rs:634`  
问题描述（一句话）：packet signature 把 `source` 文件路径当成结构身份的一部分。  
为什么是坏味道：文件重命名/移动是代码整理，不该变成协议身份变化。  
影响范围：重构目录时可能触发 packet id 变化，影响固件/主机兼容与历史数据对齐。  
Linus式修复方向：签名仅保留结构语义（struct/type/packed/byte_size/fields），将 `source` 作为审计元数据，不参与 ID 身份。  

### 问题ID：RA-003

级别：P1 + 味道评分 🟡  
证据：`crates/rat-sync/src/lib.rs:430`、`crates/rat-sync/src/lib.rs:473`、`crates/rat-sync/src/lib.rs:494`  
问题描述（一句话）：结构体布局由解析器近似推导（`contains("packed")` + 手工对齐），存在 ABI 漂移风险。  
为什么是坏味道：运行时协议正确性依赖“编译器真实布局”，而不是“解析器推测布局”。  
影响范围：跨编译器/跨目标平台时可能出现 payload 误解码。  
Linus式修复方向：短期先把支持边界写死并在 `sync` 输出显式告警；中期引入编译产物校验路径（例如由编译器导出的布局信息）。  

### 问题ID：RA-004

级别：P1 + 味道评分 🟡  
证据：`crates/rttd/src/main.rs:56`、`crates/rttd/src/main.rs:272`、`crates/rat-config/src/lib.rs:237`  
问题描述（一句话）：`reader_buf` 对外可配，但运行时无任何效果。  
为什么是坏味道：公开接口与实际行为不一致，会制造假控制面。  
影响范围：CLI/TOML 用户调参无效，排障时误判严重。  
Linus式修复方向：二选一，立即接入真实 reader buffer 或删除该参数并同步文档。  

### 问题ID：RA-005

级别：P2 + 味道评分 🟡  
证据：`crates/rttd/src/main.rs:634`、`crates/rttd/src/main.rs:637`  
问题描述（一句话）：Unknown packet 仅 warn+drop，缺少结构化统计与告警升级策略。  
为什么是坏味道：问题能看到，但不可运营。  
影响范围：长时间运行时，漂移问题会被日志淹没。  
Linus式修复方向：增加计数器与阈值触发（例如 N 秒内超过阈值直接标红或退出）。  

### 问题ID：RA-006

级别：P2 + 味道评分 🟡  
证据：`crates/rat-bridge-foxglove/src/lib.rs:493`、`crates/rat-bridge-foxglove/src/lib.rs:506`、`docs/workflow.md:52`、`docs/quickstart.md:108`  
问题描述（一句话）：image 通道是“字段派生图像”，不是设备原始像素 payload。  
为什么是坏味道：实现本身没错，但外部预期很容易被“图像流”措辞误导。  
影响范围：用户会把 mock/派生图像当成真实采图能力，导致错误验收。  
Linus式修复方向：文档统一使用“派生图像帧（derived image）”术语，并在 CLI 启动日志中明确声明。  

### 问题ID：RA-007

级别：P3 + 味道评分 🟢  
证据：`crates/rttd/src/main.rs:171`、`crates/rttd/src/main.rs:183`  
问题描述（一句话）：`server/foxglove` 默认自动 `sync`，属于带副作用启动。  
为什么是坏味道：不是 bug，但会增加“命令即修改文件”的认知负担。  
影响范围：CI/只读环境/审计流程可能需要显式关闭副作用。  
Linus式修复方向：保留默认自动同步，同时增加显式开关（例如 `--no-auto-sync`）以便可控运行。  

## 7. 味道评分总览

总体评分：🟡（中等）  
评分依据：

1. 主链路架构方向正确，legacy 大部分已清理（加分）。
2. 仍存在 1 个致命入口一致性缺口（扣分）。
3. 还留有 2 个会积累兼容债的结构问题（扣分）。

## 8. 致命问题（Fatal）

1. RA-001（P0）：init fingerprint 未做硬校验。  
2. 这是“现在不炸、以后必炸”的问题，因为它把兼容错误变成运行期随机现象。  

## 9. 改进方向（按优先级）

1. 先修 RA-001：把 fingerprint 比对变成启动硬门禁。  
2. 再修 RA-002：签名去路径化，稳定协议身份。  
3. 同步修 RA-004：删或接 `reader_buf`，消灭假配置面。  
4. 处理 RA-003：明确 ABI 边界并加可观测告警。  
5. 收尾 RA-005/RA-006：补运营指标与文案一致性。  

## 10. 分阶段整改路线图（D0/D1/D2）

### D0（当天，阻断风险）

1. 在 `rttd` 建立 expected fingerprint 与 init 包比对机制，不一致直接失败退出。  
2. 给 `server` 和 `foxglove` 复用同一错误文案与退出码。  
3. 验收：构造 fingerprint 不一致样例，确认启动被拒绝。  

依赖：无  
预估收益：立即消除“静默漂移”。

### D1（1~2 天，结构收敛）

1. 从 `compute_signature_hash` 移除 `source` 参与。  
2. 提供一次性迁移策略（兼容旧 `rat_gen.toml` 的映射提示或自动重建说明）。  
3. 处理 `reader_buf`：要么真正接线，要么删除参数并更新文档。  
4. 验收：重命名文件不再导致 packet id 非预期变化；CLI 参数行为与文档一致。  

依赖：D0 完成  
预估收益：降低重构导致协议抖动的概率，减少维护成本。

### D2（3~5 天，观测性与文档收口）

1. 给 unknown packet 增加计数和阈值升级机制。  
2. 文档将 image 统一表述为“派生图像帧”，避免能力误读。  
3. 补一条端到端验收脚本：`sync -> firmware/mock -> server/foxglove`，覆盖 mismatch 场景。  
4. 验收：异常场景可被快速定位，文档与实现口径一致。  

依赖：D1 完成  
预估收益：降低线上排障成本，提升系统可解释性。

## 11. 附录（命令与证据摘要）

执行过的关键命令：

1. `git status --short`
2. `git submodule status`
3. `cargo test -q`
4. `nl -ba <file> | sed -n ...`（逐模块证据定位）

测试采样结果：

1. `cargo test -q` 全部通过（当前基线未见单元测试阻断项）。

一致性正向证据：

1. `server/foxglove` 同口径 fail-fast：`crates/rttd/src/main.rs:241`、`crates/rttd/src/main.rs:334`。  
2. 协议模型已收敛：`crates/rat-protocol/src/lib.rs:39`。  
3. 固件示例链路完整：`firmware/example/stm32f4_rtt/Core/Src/main.c:47`、`firmware/example/stm32f4_rtt/Core/Src/main.c:130`、`firmware/example/stm32f4_rtt/rat_gen.h:8`。  

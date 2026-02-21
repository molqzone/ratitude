# rttd 工作流总览（v0.2）

本文说明从 C 声明到主机运行的单入口 daemon 流程。

## 1. 端到端流程

```text
C 源码(@rat) -> 内部同步生成 rat_gen.toml + rat_gen.h
           -> 固件编译并 emit -> 已存在 RTT 端点
           -> rttd daemon -> JSONL / Foxglove
```

## 2. 启动阶段

执行：

```bash
cargo run -p rttd -- --config <path/to/rat.toml>
```

启动行为：

1. 读取 `rat.toml`
2. 扫描 source 候选地址（`auto_scan=false` 时仅探测 `last_selected_addr`）
3. 选择 source 并持久化
4. 触发启动自动同步（可在配置关闭）
5. 加载 `rat_gen.toml` 并校验指纹
6. 启动 ingest runtime

## 3. 运行阶段

命令台控制：

- `$help` 查看命令
- `$status` 查看状态
- `$source list` 列出候选源
- `$source use <index>` 切换源并重启 runtime
- `$sync` 手动触发同步
- `$foxglove on|off` 控制 Foxglove 输出
- `$jsonl on|off [path]` 控制 JSONL 输出
- `/packet/<struct>/<field>` 查看字段元数据

持久化规则：

- 命令台是主配置入口之一。
- `$source list` 与 `$source use` 每次执行前都会实时刷新候选源，可达性不是启动快照。
- 候选索引会随刷新结果变化；`$source use <index>` 以刷新后的列表为准。
- `$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。
- 运行时重启会重建 JSONL writer；当 `$jsonl on <path>` 生效时，目标文件按清空重写处理（非追加）。
- `$sync` 只更新生成物（`rat_gen.toml`/`rat_gen.h`），不写 `rttd.outputs`/`rttd.source` 运行配置。

## 4. 同步与一致性

- 同步语义由 daemon 内部控制，不暴露外部子命令。
- init/reset 事件可触发防抖单飞同步（可配置关闭）。
- `rat_gen.toml` 缺失、空包或指纹不一致均 fail-fast。

## 5. 常见问题

### 启动失败并提示 generated 文件问题

- 检查 `generation.out_dir`、`toml_name`、`header_name`
- 检查 `@rat` 声明是否可被扫描
- 在 daemon 中执行 `$sync` 后重试

### 启动后无数据

- 检查 source 选择是否正确（`$source list`）
- 检查固件是否调用 `rat_init()` 与持续 `rat_emit`

### 指纹不一致

- 触发 `$sync`
- 重新编译并烧录固件，确保 `rat_gen.h` 与主机一致

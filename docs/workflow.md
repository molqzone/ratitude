# ratd 工作流总览

本文说明固定流程：先 `ratsync` 生成头文件，再编译烧录，最后启动 `ratd` 进入运行时解码。

## 1. 端到端流程

```text
ratsync 生成 rat_gen.h
         -> build/flash firmware
         -> 固件启动，RTT 端点可连接
         -> ratd daemon 连接 source
         -> firmware 发送 schema 控制帧（HELLO/CHUNK/COMMIT）
         -> rat-core runtime Ready
         -> JSONL / Foxglove
```

模块边界：

```text
ratd(console/source/output) -> rat-core(runtime)
```

## 2. 同步与构建阶段

执行：

```bash
cargo run -p ratsync -- --config <path/to/rat.toml>
# build + flash firmware
```

说明：

- `ratsync` 是唯一同步入口。
- `ratd` 不会触发 sync，不会生成 `rat_gen.h`。

## 3. 启动阶段

执行：

```bash
cargo run -p ratd -- --config <path/to/rat.toml>
```

启动行为：

1. 读取 `rat.toml`
2. 扫描 source 候选地址（`auto_scan=false` 时仅探测 `last_selected_addr`）
3. 选择 source 并持久化
4. 启动 ingest runtime（初始状态 `WaitingSchema`）
5. 在 `schema_timeout` 窗口内等待 schema 控制帧

## 4. 运行阶段

命令台控制：

- `$help` 查看命令
- `$status` 查看状态
- `$source list` 列出候选源
- `$source use <index>` 切换源并重启 runtime
- `$foxglove on|off` 控制 Foxglove 输出
- `$jsonl on|off [path]` 控制 JSONL 输出
- `/packet/<struct>/<field>` 查看字段元数据

持久化规则：

- 命令台是主配置入口之一。
- `$source list` 与 `$source use` 每次执行前都会实时刷新候选源，可达性不是启动快照。
- 候选索引会随刷新结果变化；`$source use <index>` 以刷新后的列表为准。
- `$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。
- 运行时重启会重建 JSONL writer；当 `$jsonl on <path>` 生效时，目标文件按清空重写处理（非追加）。

## 5. Schema 一致性

- runtime 在 `WaitingSchema` 状态时不会解码业务包。
- 收到完整 schema 并校验 hash 后进入 `Ready`。
- schema 超时或 hash 不一致直接 fail-fast。
- unknown packet 监控与阈值告警在 `Ready` 后生效。

## 6. 常见问题

### 启动后报 schema timeout

- 检查 RTT source 是否连接到了正确端口（`$source list`）。
- 检查固件是否发送 schema 控制帧。
- 检查 `ratd.behavior.schema_timeout` 是否过短。

### 启动后无数据

- 检查 source 选择是否正确（`$source list`）。
- 检查固件是否持续发送业务包。
- 检查命令台是否已切换正确输出（`$jsonl` / `$foxglove`）。

### schema hash 不一致

- 先执行 `ratsync`，然后重新编译并烧录固件。
- 确认固件发送的是完整 schema 数据。
- 确认 CHUNK 顺序和长度正确。

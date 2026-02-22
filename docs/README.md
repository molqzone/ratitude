# ratd 文档导航

推荐阅读顺序：

1. [快速开始](./quickstart.md)
2. [工作流总览](./workflow.md)
3. [配置文件说明](./config-files.md)
4. [@rat 声明规范](./rat-annotation-spec.md)
5. [路线图](./roadmap.md)

## 新运行模型（v0.2）

运行顺序固定为：

```bash
cargo run -p ratsync -- --config firmware/example/stm32f4_rtt/rat.toml
# build + flash firmware
cargo run -p ratd -- --config firmware/example/stm32f4_rtt/rat.toml
```

其中 `ratd` 只保留单入口交互式 daemon，不触发 sync。

启动后通过命令台控制行为：

- `$help`
- `$status`
- `$source list`
- `$source use <index>`
- `$foxglove on|off`
- `$jsonl on|off [path]`
- `/packet/<struct>/<field>`

说明：

- 命令台是主配置入口之一。
- `$source use`、`$foxglove on|off`、`$jsonl on|off [path]` 会持久化写回 `rat.toml`。

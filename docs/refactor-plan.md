# ratd daemon breaking refactor plan (0.2.0)

## 目标

- 单入口 daemon
- 删除外部子命令痕迹
- 新配置模型落地
- 同步机制内部化

## 实施要点

1. 配置层升级到 `source/behavior/outputs`
2. 同步层接入 `.rttdignore`
3. 运行时拆分：`cli/daemon/console/source_scan/sync_controller/output_manager`
4. 文档与脚本改口径
5. 测试与 grep 验收

## 验收命令

```bash
cargo test -p rat-config
cargo test -p rat-sync
cargo test -p ratd
rg -n "\\bratd\\s+(server|foxglove|sync)\\b" README.md docs Makefile tools crates/ratd/src
```

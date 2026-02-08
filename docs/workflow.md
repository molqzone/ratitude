# rttd 工作流总览

本文解释：从 C 声明到主机解码，rttd 到底做了什么。

## 1. 端到端流程

```text
C 源码(@rat) -> rttd sync -> rat_gen.toml + rat_gen.h
           -> 固件编译并 emit -> RTT backend(OpenOCD/J-Link)
           -> rttd server/foxglove -> JSONL/Foxglove
```

## 2. 各阶段职责

### 阶段 A：声明扫描与生成（离线）

执行命令：

```bash
rttd sync --config <path/to/rat.toml>
```

行为：

1. 扫描 `project.scan_root` 下源码文件
2. 找出 `@rat` 注释对应的 `typedef struct`
3. 计算结构签名哈希
4. 自动分配 packet id（哈希起点 + 冲突线性探测）
5. 生成：
   - `rat_gen.toml`（主机解码定义）
   - `rat_gen.h`（固件 packet id 宏 + 指纹宏）

### 阶段 B：固件运行（在线）

行为：

1. 固件调用 `rat_init()`
2. `librat` 发送 init magic 包（包含 `RAT_GEN_FINGERPRINT`）
3. 固件持续 `rat_emit(...)` 发送业务数据

### 阶段 C：主机接收（在线）

行为：

1. `rttd` 从 RTT TCP 地址读取字节流
2. 去除 J-Link banner（若启用 J-Link backend）
3. COBS 解码得到 `id + payload`
4. 若是 init magic：记录日志并跳过业务解析
5. 否则按 `rat_gen.toml` 的字段布局动态解析
6. `foxglove` 模式严格按 `rat_gen.toml` 发布声明驱动通道（逐包 topic/schema）
7. `image` 类型额外派生 `/rat/{struct_name}/image` 真图像流（RawImage）

### 阶段 D：无硬件联调（OpenOCD 字节流 mock）

执行命令：

```bash
./tools/run_mock_foxglove.sh
```

行为：

1. 启动 `tools/openocd_rtt_mock.py`，监听 `127.0.0.1:19021`
2. 按 `examples/mock/rat_gen.toml` 持续生成 COBS 帧（`[id+payload]+0x00`）
3. 启动 `rttd foxglove` 消费该字节流并转发至 Foxglove
4. Ctrl+C 级联停止 mock 与 rttd

## 3. 你最常用的命令

```bash
# 生成阶段
rttd sync --config firmware/example/stm32f4_rtt/rat.toml

# 在线运行（JSONL）
rttd server --config firmware/example/stm32f4_rtt/rat.toml --log out.jsonl

# 无硬件联调（Foxglove）
./tools/run_mock_foxglove.sh
```

## 4. 常见问题定位

### sync 后 packet 仍为 0

优先检查：

- 注释是否使用新语法 `@rat, <type>`
- 注释后是否紧跟 `typedef struct`
- `scan_root` 和 `extensions` 是否正确

### 收到 RTT 连接但没有有效包

优先检查：

- 固件是否调用了 `rat_init()`
- 固件是否在持续 `rat_emit`
- 包结构是否与 `rat_gen.toml` 一致

### foxglove 启动即失败

优先检查：

- `rat_gen.toml` 是否存在
- `rat_gen.toml` 的 `packets` 是否为空
- `[rttd.foxglove]` 是否仅包含 `ws_addr`

### mock 服务启动失败

优先检查：

- `examples/mock/rat_gen.toml` 是否存在且 `packets` 非空
- mock 端口 `127.0.0.1:19021` 是否被占用
- Python 版本是否可用（建议 Python 3.11+）

# 快速开始（5 分钟）

本文目标：让你在不了解系统内部细节的前提下，先跑通一次完整链路。

## 前置条件

- 已安装 Rust 与 Cargo
- 已有可运行的固件工程（示例：`firmware/example/stm32f4_rtt`）
- 有可用 RTT backend（OpenOCD 或 J-Link）

## 第 1 步：写声明（或确认已有声明）

在 C 文件里写 `@rat` 标注，紧跟一个 `typedef struct`：

```c
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;
```

说明：

- `@rat` 可以不写类型，默认是 `plot`
- 旧语法 `@rat:id=...` / `@rat, type=...` 已废弃
- `@rat` 结构体不支持 `aligned(...)` / `#pragma pack` 自定义对齐；非 `packed` 且存在 padding/8字节字段风险会被 `sync` 直接阻断

## 第 2 步：准备配置

确认 `rat.toml` 存在（默认路径是当前目录 `./rat.toml`），例如：

- `firmware/example/stm32f4_rtt/rat.toml`

关键字段：

- `project.scan_root`：扫描目录
- `generation.out_dir`：`rat_gen.toml` / `rat_gen.h` 输出目录
- `rttd.server.addr`：rttd 连接 RTT 的地址

## 第 3 步：生成 rat_gen.*

```bash
cargo run -p rttd -- sync --config firmware/example/stm32f4_rtt/rat.toml
```

生成结果：

- `rat_gen.toml`（主机用）
- `rat_gen.h`（固件用）

## 第 4 步：固件侧接入

- 在工程中包含 `rat_gen.h`
- 发包时使用生成的包 ID 宏（例如 `RAT_ID_RATSAMPLE`）
- `rat_init()` 会发送 init magic（含配置指纹）给 rttd

## 第 5 步：启动 backend + rttd

### J-Link

```bash
./tools/jlink_rtt_server.sh --device STM32F407ZG --if SWD --speed 4000 --rtt-port 19021
cargo run -p rttd -- server --config firmware/example/stm32f4_rtt/rat.toml --log out.jsonl
```

### OpenOCD

```bash
powershell -ExecutionPolicy Bypass -File tools/openocd_rtt_server.ps1
cargo run -p rttd -- server --config firmware/example/stm32f4_rtt/rat.toml --log out.jsonl
```

## 第 6 步：验证成功

满足以下任一项即可：

- 终端看到 `received librat init magic packet`
- `out.jsonl` 持续增长
- `foxglove` 模式下能看到实时数据

## 运行模式说明（声明驱动）

- `rttd server` 与 `rttd foxglove` 都严格依赖 `rat_gen.toml`
- `rat_gen.toml` 缺失或 `packets=[]` 时，两种模式都会直接失败
- `rttd foxglove` 仅保留运行参数：`--config --addr --ws-addr --reconnect --buf --backend* --openocd* --jlink*`
- 数据通道和 schema 完全由 `rat_gen.toml` 决定

## 无硬件 Mock 联调（推荐先验链路）

### 一键启动

```bash
./tools/run_mock_foxglove.sh
```

### 手动启动

```bash
python -X utf8 tools/openocd_rtt_mock.py --config examples/mock/rat.toml --verbose
cargo run -p rttd -- foxglove --config examples/mock/rat.toml --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765 --backend none --no-auto-start-backend --no-auto-sync
```

说明：

- mock 数据包严格来自 `examples/mock/rat_gen.toml`
- 默认平衡频率：`quat=50Hz`、`waveform=50Hz`、`temperature=5Hz`、`image=2Hz`
- `image` 包会自动派生 `/rat/{struct_name}/image` 派生图像帧（foxglove.RawImage, mono8，非原始 payload 图像字节）
- mock 默认用 `width/height/frame_idx/luma` 字段生成动态图像帧

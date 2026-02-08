# Ratitude

> High-performance binary telemetry stack for embedded systems.

Ratitude provides a low-latency RTT telemetry pipeline:

- Firmware-side `librat` emits binary C structs with COBS framing.
- Host-side `rttd` (Rust) receives streams, decodes packets, and routes data to JSONL or Foxglove.

## Core capabilities

- Binary struct transport instead of printf text formatting.
- COBS framing for reliable packet boundaries over byte streams.
- Rust host pipeline (`rat-core -> rat-protocol -> rttd -> logger/bridge`).
- JSONL output for offline analysis.
- OpenOCD RTT compatible transport path.
- J-Link RTT compatible transport path.
- Config-driven runtime packet decoding from C annotations.

### Host architecture

- `rat-core`: transport listener + hub + JSONL writer runtime primitives
- `rat-protocol`: COBS + packet parsing and protocol context
- `rat-sync`: `@rat` scanner and generated files sync
- `rat-config`: config model and TOML persistence
- `rat-bridge-foxglove`: Foxglove bridge/channels
- `rttd`: CLI orchestration (`server` / `foxglove` / `sync`)

## 文档入口

- 新手阅读建议：`docs/README.md`
- 5 分钟上手：`docs/quickstart.md`
- 工作流总览：`docs/workflow.md`

## Build

```bash
cargo build -p rttd
```

## Config-driven initialization (C -> TOML)

`@rat` annotations in C are the single source of truth for packet definitions.

- `rat.toml` stores project/runtime settings (scan scope, artifacts, backend options).
- `rat_gen.toml` is auto-generated from `@rat` declarations and should not be edited manually.
- `rat_gen.h` is auto-generated for firmware packet IDs and fingerprint.
- Runtime precedence: `flags > TOML > built-in defaults`.

### C annotation format

`@rat` supports line comments and block comments:

```c
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;

/* @rat, log */
typedef struct {
  uint16_t voltage_mv;
  uint16_t current_ma;
} BatteryReading;
```

`@rat` also supports omitted type (`// @rat`), which defaults to `plot`.

Supported `type` values:

- `plot`
- `quat`
- `image`
- `log`

Scanner backend uses Rust `tree-sitter` + `tree-sitter-c` AST to locate `typedef struct` and fields.

### Manual sync

```bash
rttd sync --config firmware/example/stm32f4_rtt/rat.toml
```

### Start with auto-sync (recommended)

Both commands auto-sync packets before startup:

```bash
rttd server --config firmware/example/stm32f4_rtt/rat.toml
rttd foxglove --config firmware/example/stm32f4_rtt/rat.toml
```

## RTT backend startup (OpenOCD / J-Link)

`rttd` consumes framed RTT bytes from `--addr` (default `127.0.0.1:19021`).
Backend process should be started first, then `rttd` attaches to the TCP endpoint.

### OpenOCD RTT server

```bash
powershell -ExecutionPolicy Bypass -File tools/openocd_rtt_server.ps1
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

### J-Link RTT server

```bash
./tools/jlink_rtt_server.sh --device STM32F407ZG --if SWD --speed 4000 --rtt-port 19021
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

On Windows:

```powershell
powershell -ExecutionPolicy Bypass -File tools/jlink_rtt_server.ps1 -Device STM32F407ZG -Interface SWD -Speed 4000 -RttTelnetPort 19021
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

You can also let `rttd` auto-start backend via config/flags (`--backend`, `--auto-start-backend`).

When `--backend jlink` is selected, `rttd` strips the SEGGER RTT banner line before COBS frame decoding.

## `rttd server`

```bash
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

Common flags:

- `--config`: TOML config path
- `--addr`: TCP source address
- `--log`: JSONL output file path (default stdout)
- `--text-id`: text packet id
- `--reconnect`: reconnect interval (example: `1s`)
- `--buf`: frame channel buffer size
- `--reader-buf`: reader buffer size
- `--backend`: backend type (`none` / `openocd` / `jlink`)
- `--auto-start-backend`: let `rttd` auto-start backend process
- `--no-auto-start-backend`: force disable backend auto-start
- `--backend-timeout-ms`: backend startup wait timeout
- `--openocd-*`: override OpenOCD backend options
- `--jlink-*`: override J-Link backend options

## `rttd foxglove`

```bash
rttd foxglove --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765
```

Common flags:

- `--config`: TOML config path
- `--addr`: RTT TCP source address
- `--ws-addr`: WebSocket listen address
- `--reconnect`: reconnect interval
- `--buf`: frame channel buffer size
- `--backend` / `--auto-start-backend` / `--no-auto-start-backend` / `--backend-timeout-ms`
- `--openocd-*` / `--jlink-*`

Foxglove mode is strictly declaration-driven:

- Data channels and schemas are generated only from `rat_gen.toml`.
- Fixed legacy channels are removed.
- Unknown packet IDs are dropped.
- If `rat_gen.toml` is missing or `packets=[]`, startup fails immediately.

## Foxglove channels

The bridge uses the official `foxglove` Rust SDK and publishes declaration-driven channels:

- Data topic: `/rat/{struct_name}`
- Schema name: `ratitude.{struct_name}`
- For every `quat` packet: additional `/rat/{struct_name}/marker` and `/rat/{struct_name}/tf`
- For every `image` packet: additional `/rat/{struct_name}/image` (foxglove.RawImage, mono8)

Open Foxglove, connect to `ws://127.0.0.1:8765`, then subscribe in panels.

## OpenOCD-like mock for Foxglove integration

For local testing without hardware, use the independent declaration-driven mock tools:

- Python byte-stream mock server: `tools/openocd_rtt_mock.py`
- One-click startup (Linux/macOS): `tools/run_mock_foxglove.sh`
- One-click startup (Windows PowerShell): `tools/run_mock_foxglove.ps1`
- Mock config package: `examples/mock/rat.toml` + `examples/mock/rat_gen.toml`

Run one-click flow:

```bash
./tools/run_mock_foxglove.sh
```

Or run manually:

```bash
python -X utf8 tools/openocd_rtt_mock.py --config examples/mock/rat.toml --verbose
cargo run -p rttd -- foxglove --config examples/mock/rat.toml --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765 --backend none --no-auto-start-backend
```

## Make targets

```bash
make sync
make server
make foxglove
make mock-foxglove
make up
```

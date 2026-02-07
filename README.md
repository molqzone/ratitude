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
- Config-driven runtime packet decoding from C annotations.

### Host architecture

- `rat-core`: transport listener + hub + JSONL writer runtime primitives
- `rat-protocol`: COBS + packet parsing and protocol context
- `rat-sync`: `@rat` scanner and TOML packet sync
- `rat-config`: config model and TOML persistence
- `rat-bridge-foxglove`: Foxglove bridge/channels
- `rttd`: CLI orchestration (`server` / `foxglove` / `sync`)

## Build

```bash
cargo build -p rttd
```

## Config-driven initialization (C -> TOML)

`@rat` annotations in C are the single source of truth for packet definitions.

- `[[packets]]` is auto-managed and overwritten by scanner sync.
- `[rttd.server]` / `[rttd.foxglove]` are manual runtime settings and are preserved.
- Runtime precedence: `flags > TOML > built-in defaults`.

### C annotation format

`@rat` supports line comments and block comments:

```c
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;

/* @rat:id=0x02, type=json */
typedef struct {
  uint16_t voltage_mv;
  uint16_t current_ma;
} BatteryReading;
```

Scanner backend uses Rust `tree-sitter` + `tree-sitter-c` AST to locate `typedef struct` and fields.

### Manual sync

```bash
rttd sync --config firmware/example/stm32f4_rtt/ratitude.toml
```

### Start with auto-sync (recommended)

Both commands auto-sync packets before startup:

```bash
rttd server --config firmware/example/stm32f4_rtt/ratitude.toml
rttd foxglove --config firmware/example/stm32f4_rtt/ratitude.toml
```

### Default config file

- `firmware/example/stm32f4_rtt/ratitude.toml`

### Path resolution rules

- Relative paths from TOML (for example `rttd.foxglove.image_path = '../../../demo.jpg'`) are resolved relative to the config file directory.
- Paths passed via CLI flags keep standard CLI behavior and are resolved from the current working directory.

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
- `--reader-buf`: retained for compatibility

## `rttd foxglove`

```bash
rttd foxglove --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765
```

Common flags:

- `--config`: TOML config path
- `--ws-addr`: WebSocket listen address
- `--topic`: generic packet topic
- `--schema-name`: generic packet schema name
- `--marker-topic`: marker topic for 3D panel
- `--quat-id`: quaternion packet id override
- `--temp-id`: temperature packet id
- `--parent-frame`: transform parent frame id
- `--frame-id`: marker frame id / transform child frame id
- `--image-path`: compressed image file path (CLI path uses current working directory)
- `--image-frame`: frame id for image stream
- `--image-format`: compressed image format tag
- `--log-topic`: Foxglove Log Panel topic
- `--log-name`: source name in log records
- `--mock`: enable local mock packets
- `--mock-hz`: mock sample rate
- `--mock-id`: mock quaternion packet id

## Foxglove channels

The bridge uses the official `foxglove` Rust SDK and publishes six channels:

- `ratitude/packet`
- `/ratitude/log`
- `/ratitude/temperature`
- `/visualization_marker`
- `/tf`
- `/camera/image/compressed`

Open Foxglove, connect to `ws://127.0.0.1:8765`, then subscribe in panels.

Image payload loading is asynchronous during bridge startup. If image loading fails, only `/camera/image/compressed` is disabled; other channels continue normally.

## Make targets

```bash
make sync
make server
make foxglove
make up
```


# Ratitude

> **High-performance binary telemetry stack for embedded systems.**

Ratitude provides a low-latency RTT telemetry pipeline:

- Firmware-side `librat` emits binary C structs with COBS framing.
- Host-side `rttd` receives streams, decodes packets, and routes data to logs or Foxglove.

## Core capabilities

- Binary struct transport instead of printf text formatting.
- COBS framing for reliable packet boundaries over byte streams.
- Go host pipeline (`transport` -> `protocol` -> `engine` -> `logger`).
- JSONL output for offline analysis.
- OpenOCD RTT compatible transport path.
- Config-driven runtime packet decoding from C annotations.

## Quick start

```bash
git submodule update --init --recursive
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

## Config-driven initialization (C -> TOML)

`@rat` annotations in C are the single source of truth for packet definitions.

- `[[packets]]` is **auto-managed** and overwritten by scanner sync.
- `[rttd.server]` / `[rttd.foxglove]` are **manual runtime settings** and are preserved.
- Runtime precedence is: **flags > TOML > built-in defaults**.

### C annotation format

`@rat` can be written in line comments or block comments:

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

The scanner uses Tree-sitter C AST (CGO build) to locate `typedef struct` and fields.

### Start with auto-sync (recommended)

```bash
rttd server --config firmware/example/stm32f4_rtt/ratitude.toml
rttd foxglove --config firmware/example/stm32f4_rtt/ratitude.toml
```

Both commands run packet sync before startup, so firmware developers only need to:

1. write C structs + `@rat`
2. run CLI command

### Manual sync (optional, for CI/debug)

> CGO note: Tree-sitter parsing requires a C toolchain when `CGO_ENABLED=1`.
> In non-CGO environments, Ratitude falls back to compatibility parsing.

```bash
go run tools/rat-gen.go sync --config firmware/example/stm32f4_rtt/ratitude.toml
```

To validate the Tree-sitter path with CGO enabled:

```powershell
./tools/test_cgo.ps1 -Packages ./...
```

### Default config file

- `firmware/example/stm32f4_rtt/ratitude.toml`

### `rttd server` common flags

- `--config`: TOML config path (default `firmware/example/stm32f4_rtt/ratitude.toml`)
- `--addr`: TCP source address (default from TOML)
- `--log`: JSONL output file path (default stdout)
- `--text-id`: text packet id (default from TOML)

## Foxglove bridge

```bash
rttd foxglove --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765
```

### `rttd foxglove` common flags

- `--config`: TOML config path (default `firmware/example/stm32f4_rtt/ratitude.toml`)
- `--ws-addr`: WebSocket listen address (default from TOML)
- `--topic`: generic packet topic (default from TOML)
- `--schema-name`: generic packet schema name (default from TOML)
- `--marker-topic`: marker topic for 3D panel (default from TOML)
- `--quat-id`: quaternion packet id (explicit override). If omitted and TOML `quat_id` does not match discovered packets, first `type=pose_3d` packet is used.
- `--temp-id`: temperature packet id (default from TOML, payload is `struct { float celsius; }`)
- `--parent-frame`: transform parent frame id (default from TOML)
- `--frame-id`: marker frame id / transform child frame id (default from TOML)
- `--image-path`: compressed image file used for image stream (default from TOML)
- `--image-frame`: frame id for image stream (default from TOML)
- `--image-format`: compressed image format tag (default from TOML)
- `--log-topic`: Foxglove Log Panel topic (default from TOML)
- `--log-name`: source name in log records (default from TOML)

### IMU-style tri-axis mock source

```bash
rttd foxglove --mock --mock-hz 50 --mock-id 0x10
```

- `--mock`: generate local mock packets instead of TCP input (XYZ tri-axis sinusoidal motion)
- `--mock-hz`: mock sample rate (default `50`)
- `--mock-id`: mock quaternion packet id (default `0x10`)
- mock mode also emits `rat_info`-style text once per second on `--text-id` for Log Panel testing
- mock mode emits temperature packets on `--temp-id` for Gauge panel testing

### IMU 3D visualization in Foxglove

The bridge publishes six JSON channels:

- `ratitude/packet`: normalized packet stream
- `/ratitude/log`: `foxglove.Log` stream generated from `rat_info` text packets
- `/ratitude/temperature`: temperature stream for Gauge panel (`value` in Celsius)
- `/visualization_marker`: white CUBE marker driven by quaternion packets
- `/tf`: `foxglove.FrameTransforms` for frame tree (`world` -> `base_link`)
- `/camera/image/compressed`: repeated `foxglove.CompressedImage` frames from `demo.jpg`

Open Foxglove, connect to `ws://127.0.0.1:8765`, add a **3D Panel**, and subscribe to `/visualization_marker`.

For text logs, add a **Log Panel** and subscribe to `/ratitude/log`.

For a temperature gauge mock, add a **Gauge Panel**, subscribe to `/ratitude/temperature`, and set message path to `value`.

For the image stream, add an **Image Panel** and subscribe to `/camera/image/compressed`.

The mock source rotates on roll/pitch/yaw together (not a single-axis spin).






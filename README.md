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

## Quick start

```bash
git submodule update --init --recursive
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```

### `rttd server` common flags

- `--addr`: TCP source address (default `127.0.0.1:19021`)
- `--log`: JSONL output file path (default stdout)
- `--text-id`: text packet id (default `0xFF`)

## Foxglove bridge

```bash
rttd foxglove --addr 127.0.0.1:19021 --ws-addr 127.0.0.1:8765
```

### `rttd foxglove` common flags

- `--ws-addr`: WebSocket listen address (default `127.0.0.1:8765`)
- `--topic`: generic packet topic (default `ratitude/packet`)
- `--schema-name`: generic packet schema name (default `ratitude.Packet`)
- `--marker-topic`: marker topic for 3D panel (default `/visualization_marker`)
- `--quat-id`: quaternion packet id (default `0x10`, payload is `struct { float w, x, y, z; }`)
- `--temp-id`: temperature packet id (default `0x20`, payload is `struct { float celsius; }`)
- `--parent-frame`: transform parent frame id (default `world`)
- `--frame-id`: marker frame id / transform child frame id (default `base_link`)
- `--image-path`: compressed image file used for image stream (default `D:/Repos/ratitude/demo.jpg`, set to empty to disable)
- `--image-frame`: frame id for image stream (default `camera`)
- `--image-format`: compressed image format tag (default `jpeg`)
- `--log-topic`: Foxglove Log Panel topic (default `/ratitude/log`)
- `--log-name`: source name in log records (default `ratitude`)

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

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
- `--parent-frame`: transform parent frame id (default `world`)
- `--frame-id`: marker frame id / transform child frame id (default `base_link`)
- `--image-path`: compressed image file used for image stream (default `D:/Repos/ratitude/demo.jpg`, set to empty to disable)
- `--image-frame`: frame id for image stream (default `camera`)
- `--image-format`: compressed image format tag (default `jpeg`)

### IMU-style tri-axis mock source

```bash
rttd foxglove --mock --mock-hz 50 --mock-id 0x10
```

- `--mock`: generate local mock packets instead of TCP input (XYZ tri-axis sinusoidal motion)
- `--mock-hz`: mock sample rate (default `50`)
- `--mock-id`: mock packet id (default `0x10`)

### IMU 3D visualization in Foxglove

The bridge publishes four JSON channels:

- `ratitude/packet`: normalized packet stream
- `/visualization_marker`: white CUBE marker driven by quaternion packets
- `/tf`: `foxglove.FrameTransforms` for frame tree (`world` -> `base_link`)
- `/camera/image/compressed`: repeated `foxglove.CompressedImage` frames from `demo.jpg`

Open Foxglove, connect to `ws://127.0.0.1:8765`, add a **3D Panel**, and subscribe to `/visualization_marker`.

For the image stream, add an **Image Panel** and subscribe to `/camera/image/compressed`.

The mock source rotates on roll/pitch/yaw together (not a single-axis spin).

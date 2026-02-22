#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""OpenOCD-like RTT stream mock server (declaration-driven).

The server listens on a TCP port and sends COBS frames:
    [packet_id + payload] -> cobs_encode -> append 0x00

On each new connection, it first sends runtime schema control frames:
    HELLO -> SCHEMA_CHUNK* -> SCHEMA_COMMIT

Schema and packet definitions are loaded from rat_gen.toml resolved by rat.toml.
"""

from __future__ import annotations

import argparse
import math
import random
import socket
import struct
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional

try:
    import tomllib  # py3.11+
except ModuleNotFoundError:  # pragma: no cover
    try:
        import tomli as tomllib  # type: ignore
    except ModuleNotFoundError as err:  # pragma: no cover
        raise SystemExit("python>=3.11 or tomli is required") from err


@dataclass
class FieldDef:
    name: str
    c_type: str
    offset: int
    size: int


@dataclass
class PacketDef:
    packet_id: int
    struct_name: str
    packet_type: str
    byte_size: int
    fields: List[FieldDef]


@dataclass
class PacketEmitter:
    packet: PacketDef
    hz: int
    next_due: float
    seq: int = 0


C_TYPE_FORMAT: Dict[str, str] = {
    "float": "<f",
    "double": "<d",
    "int8_t": "<b",
    "uint8_t": "<B",
    "int16_t": "<h",
    "uint16_t": "<H",
    "int32_t": "<i",
    "uint32_t": "<I",
    "int64_t": "<q",
    "uint64_t": "<Q",
    "bool": "<?",
    "_bool": "<?",
}

CONTROL_PACKET_ID = 0x00
CONTROL_HELLO = 0x01
CONTROL_SCHEMA_CHUNK = 0x02
CONTROL_SCHEMA_COMMIT = 0x03
CONTROL_MAGIC = b"RATS"
CONTROL_VERSION = 1


def normalize_c_type(raw: str) -> str:
    value = raw.strip().lower()
    for prefix in ("const ", "volatile "):
        if value.startswith(prefix):
            value = value[len(prefix) :]
    return value.strip()


def cobs_encode(payload: bytes) -> bytes:
    out = bytearray([0])
    code_index = 0
    code = 1

    for byte in payload:
        if byte == 0:
            out[code_index] = code
            code_index = len(out)
            out.append(0)
            code = 1
            continue

        out.append(byte)
        code += 1

        if code == 0xFF:
            out[code_index] = code
            code_index = len(out)
            out.append(0)
            code = 1

    out[code_index] = code
    return bytes(out)


def fnv1a64(payload: bytes) -> int:
    hash_value = 0xCBF29CE484222325
    for byte in payload:
        hash_value ^= byte
        hash_value = (hash_value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return hash_value


def cobs_decode(frame: bytes) -> bytes:
    out = bytearray()
    idx = 0
    total = len(frame)

    while idx < total:
        code = frame[idx]
        idx += 1
        if code == 0:
            raise ValueError("invalid COBS code 0")

        count = code - 1
        if idx + count > total:
            raise ValueError("truncated COBS frame")

        out.extend(frame[idx : idx + count])
        idx += count

        if code != 0xFF and idx < total:
            out.append(0)

    return bytes(out)


def resolve_generated_toml_path(config_path: Path) -> Path:
    raw = config_path.read_bytes()
    cfg = tomllib.loads(raw.decode("utf-8"))

    generation = cfg.get("generation", {})
    out_dir = generation.get("out_dir", ".")
    toml_name = generation.get("toml_name", "rat_gen.toml")

    base_dir = config_path.parent
    generated = Path(out_dir)
    if not generated.is_absolute():
        generated = base_dir / generated
    return generated / toml_name


def load_packets(generated_toml_path: Path) -> List[PacketDef]:
    if not generated_toml_path.exists():
        raise FileNotFoundError(f"rat_gen.toml not found: {generated_toml_path}")

    raw = generated_toml_path.read_bytes()
    cfg = tomllib.loads(raw.decode("utf-8"))
    packets = cfg.get("packets", [])
    if not packets:
        raise ValueError(f"packets is empty in {generated_toml_path}")

    parsed: List[PacketDef] = []
    for item in packets:
        packet_id = int(item["id"])
        if packet_id < 0 or packet_id > 0xFF:
            raise ValueError(f"packet id out of range: {packet_id}")

        fields_raw = item.get("fields", [])
        if not fields_raw:
            raise ValueError(f"packet has no fields: id=0x{packet_id:02X}")

        fields: List[FieldDef] = []
        byte_size = int(item["byte_size"])
        for field in fields_raw:
            fd = FieldDef(
                name=str(field["name"]),
                c_type=str(field["c_type"]),
                offset=int(field["offset"]),
                size=int(field["size"]),
            )
            c_type = normalize_c_type(fd.c_type)
            if c_type not in C_TYPE_FORMAT:
                raise ValueError(f"unsupported c_type in mock: {fd.c_type}")
            expect = struct.calcsize(C_TYPE_FORMAT[c_type])
            if expect != fd.size:
                raise ValueError(
                    f"field size mismatch {fd.name}: got {fd.size}, expect {expect}"
                )
            if fd.offset < 0 or fd.offset + fd.size > byte_size:
                raise ValueError(
                    f"field out of range {fd.name}: offset={fd.offset}, size={fd.size}, byte_size={byte_size}"
                )
            fields.append(fd)

        parsed.append(
            PacketDef(
                packet_id=packet_id,
                struct_name=str(item["struct_name"]),
                packet_type=str(item["type"]).lower(),
                byte_size=byte_size,
                fields=fields,
            )
        )

    return parsed


def quat_values(t_now: float) -> Dict[str, float]:
    roll = 0.7 * math.sin(2.0 * math.pi * 0.23 * t_now)
    pitch = 0.5 * math.sin(2.0 * math.pi * 0.31 * t_now + 0.7)
    yaw = 0.9 * math.sin(2.0 * math.pi * 0.17 * t_now + 1.1)

    cr = math.cos(roll * 0.5)
    sr = math.sin(roll * 0.5)
    cp = math.cos(pitch * 0.5)
    sp = math.sin(pitch * 0.5)
    cy = math.cos(yaw * 0.5)
    sy = math.sin(yaw * 0.5)

    w = cr * cp * cy + sr * sp * sy
    x = sr * cp * cy - cr * sp * sy
    y = cr * sp * cy + sr * cp * sy
    z = cr * cp * sy - sr * sp * cy

    norm = math.sqrt(w * w + x * x + y * y + z * z)
    inv = 1.0 if norm == 0 else 1.0 / norm
    return {"x": x * inv, "y": y * inv, "z": z * inv, "w": w * inv}


def profile_hz(packet: PacketDef, profile: str) -> int:
    ptype = packet.packet_type
    lname = packet.struct_name.lower()
    has_temp = "temp" in lname or any("temp" in f.name.lower() or "celsius" in f.name.lower() for f in packet.fields)

    if profile == "high":
        if ptype == "quat":
            return 100
        if ptype == "image":
            return 10
        if has_temp:
            return 20
        return 100

    if profile == "low":
        if ptype == "quat":
            return 10
        if ptype == "image":
            return 1
        if has_temp:
            return 2
        return 10

    # balanced
    if ptype == "quat":
        return 50
    if ptype == "image":
        return 2
    if has_temp:
        return 5
    return 50


def clamp_int(value: int, bits: int, signed: bool) -> int:
    if signed:
        low = -(1 << (bits - 1))
        high = (1 << (bits - 1)) - 1
        return max(low, min(high, value))
    modulo = 1 << bits
    return value % modulo


def field_value(
    packet: PacketDef,
    field: FieldDef,
    field_index: int,
    t_rel: float,
    seq: int,
    quat: Optional[Dict[str, float]],
    rng: random.Random,
) -> float | int | bool:
    c_type = normalize_c_type(field.c_type)
    name = field.name.lower()

    if quat is not None:
        if name in quat:
            return quat[name]
        if name.startswith("q_") and name[2:] in quat:
            return quat[name[2:]]

    if packet.packet_type == "image":
        if "width" in name:
            return 320
        if "height" in name:
            return 240
        if "frame" in name:
            return seq
        if "luma" in name or "gray" in name:
            return int((math.sin(0.3 * t_rel) * 0.5 + 0.5) * 255)

    if "temp" in name or "celsius" in name:
        return 36.5 + 3.5 * math.sin(2.0 * math.pi * 0.08 * t_rel)

    if "tick" in name or name.endswith("_ms"):
        return int(t_rel * 1000)

    if packet.packet_type == "log":
        if "level" in name:
            return (seq // 10) % 5
        if "code" in name or name.endswith("id"):
            return 1000 + seq

    base = math.sin(t_rel * (0.35 + 0.07 * field_index) + field_index * 0.5)
    drift = 0.2 * math.cos(t_rel * 0.11 + field_index * 0.17)
    noise = (rng.random() - 0.5) * 0.05
    raw = base + drift + noise

    if c_type in ("float", "double"):
        scale = 100.0 if "value" in name else 1.0
        return raw * scale

    if c_type in ("bool", "_bool"):
        return ((seq + field_index) % 2) == 0

    value_i = int(raw * 1000 + seq * 3 + field_index * 17)
    if c_type == "int8_t":
        return clamp_int(value_i, 8, signed=True)
    if c_type == "uint8_t":
        return clamp_int(abs(value_i), 8, signed=False)
    if c_type == "int16_t":
        return clamp_int(value_i, 16, signed=True)
    if c_type == "uint16_t":
        return clamp_int(abs(value_i), 16, signed=False)
    if c_type == "int32_t":
        return clamp_int(value_i, 32, signed=True)
    if c_type == "uint32_t":
        return clamp_int(abs(value_i), 32, signed=False)
    if c_type == "int64_t":
        return int(value_i)
    if c_type == "uint64_t":
        return abs(int(value_i))

    raise ValueError(f"unsupported c_type: {field.c_type}")


def write_field(payload: bytearray, field: FieldDef, value: float | int | bool) -> None:
    c_type = normalize_c_type(field.c_type)
    fmt = C_TYPE_FORMAT[c_type]

    if c_type == "float":
        packed = struct.pack(fmt, float(value))
    elif c_type == "double":
        packed = struct.pack(fmt, float(value))
    elif c_type in ("bool", "_bool"):
        packed = struct.pack(fmt, bool(value))
    else:
        packed = struct.pack(fmt, int(value))

    if len(packed) != field.size:
        raise ValueError(
            f"packed size mismatch for {field.name}: {len(packed)} != {field.size}"
        )

    payload[field.offset : field.offset + field.size] = packed


def build_payload(packet: PacketDef, t_rel: float, seq: int, rng: random.Random) -> bytes:
    payload = bytearray(packet.byte_size)
    quat = quat_values(t_rel) if packet.packet_type == "quat" else None

    for index, field in enumerate(packet.fields):
        value = field_value(packet, field, index, t_rel, seq, quat, rng)
        write_field(payload, field, value)

    return bytes(payload)


def encode_frame(packet_id: int, payload: bytes) -> bytes:
    raw = bytes([packet_id]) + payload
    return cobs_encode(raw) + b"\x00"


def encode_schema_frames(schema_bytes: bytes, chunk_size: int = 256) -> List[bytes]:
    schema_hash = fnv1a64(schema_bytes)
    frames: List[bytes] = []

    hello_payload = (
        bytes([CONTROL_HELLO])
        + CONTROL_MAGIC
        + bytes([CONTROL_VERSION])
        + struct.pack("<I", len(schema_bytes))
        + struct.pack("<Q", schema_hash)
    )
    frames.append(encode_frame(CONTROL_PACKET_ID, hello_payload))

    chunk_size = max(1, chunk_size)
    offset = 0
    while offset < len(schema_bytes):
        chunk = schema_bytes[offset : offset + chunk_size]
        chunk_payload = (
            bytes([CONTROL_SCHEMA_CHUNK])
            + struct.pack("<I", offset)
            + struct.pack("<H", len(chunk))
            + chunk
        )
        frames.append(encode_frame(CONTROL_PACKET_ID, chunk_payload))
        offset += len(chunk)

    commit_payload = bytes([CONTROL_SCHEMA_COMMIT]) + struct.pack("<Q", schema_hash)
    frames.append(encode_frame(CONTROL_PACKET_ID, commit_payload))
    return frames


def run_self_tests() -> None:
    for raw in [b"", b"\x00", b"\x00\x00", b"\x01", b"abc", b"abc\x00", b"a\x00b\x00c", bytes(range(1, 128))]:
        encoded = cobs_encode(raw)
        decoded = cobs_decode(encoded)
        assert decoded == raw, f"cobs roundtrip failed: {raw!r}"

    packet = PacketDef(
        packet_id=0x10,
        struct_name="T",
        packet_type="quat",
        byte_size=16,
        fields=[
            FieldDef("x", "float", 0, 4),
            FieldDef("y", "float", 4, 4),
            FieldDef("z", "float", 8, 4),
            FieldDef("w", "float", 12, 4),
        ],
    )
    payload = build_payload(packet, 1.0, 1, random.Random(1))
    x, y, z, w = struct.unpack("<ffff", payload)
    norm = math.sqrt(x * x + y * y + z * z + w * w)
    assert abs(norm - 1.0) < 1e-3, "quat norm check failed"

    frame = encode_frame(0x10, payload)
    decoded = cobs_decode(frame[:-1])
    assert decoded[0] == 0x10
    assert decoded[1:] == payload

    print("[self-test] ok")


def run_server(args: argparse.Namespace, packets: List[PacketDef], schema_bytes: bytes) -> int:
    rng = random.Random(args.seed)
    t0 = time.perf_counter()

    emitters = [
        PacketEmitter(
            packet=packet,
            hz=profile_hz(packet, args.profile),
            next_due=time.perf_counter(),
            seq=0,
        )
        for packet in packets
    ]

    if args.verbose:
        print(f"[mock] loaded packets={len(packets)} profile={args.profile}")
        print(f"[mock] schema bytes={len(schema_bytes)} hash=0x{fnv1a64(schema_bytes):016X}")
        for emitter in emitters:
            print(
                f"  - id=0x{emitter.packet.packet_id:02X} "
                f"name={emitter.packet.struct_name} type={emitter.packet.packet_type} hz={emitter.hz}"
            )

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind((args.host, args.port))
    server.listen(1)
    server.settimeout(0.2)

    print(f"[mock] listening on {args.host}:{args.port}")

    client: Optional[socket.socket] = None

    def close_client() -> None:
        nonlocal client
        if client is not None:
            try:
                client.close()
            except OSError:
                pass
            client = None

    try:
        while True:
            now = time.perf_counter()
            if args.duration is not None and (now - t0) >= args.duration:
                if args.verbose:
                    print("[mock] duration reached, exiting")
                break

            if client is None:
                try:
                    conn, addr = server.accept()
                    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    conn.settimeout(0.0)
                    client = conn
                    print(f"[mock] client connected: {addr[0]}:{addr[1]}")
                    schema_frames = encode_schema_frames(schema_bytes)
                    schema_send_ok = True
                    for frame in schema_frames:
                        try:
                            client.sendall(frame)
                        except (BrokenPipeError, ConnectionResetError, OSError):
                            schema_send_ok = False
                            if args.verbose:
                                print("[mock] client disconnected while sending schema")
                            close_client()
                            break
                    if not schema_send_ok:
                        continue
                    if args.verbose:
                        print(
                            f"[mock] sent runtime schema frames={len(schema_frames)} hash=0x{fnv1a64(schema_bytes):016X}"
                        )
                    if args.once:
                        t_rel = now - t0
                        for emitter in emitters:
                            payload = build_payload(emitter.packet, t_rel, emitter.seq, rng)
                            emitter.seq += 1
                            frame = encode_frame(emitter.packet.packet_id, payload)
                            client.sendall(frame)
                        print("[mock] once mode sent one frame per packet")
                        break
                except socket.timeout:
                    continue

            if client is None:
                continue

            for emitter in emitters:
                if now < emitter.next_due:
                    continue

                t_rel = now - t0
                payload = build_payload(emitter.packet, t_rel, emitter.seq, rng)
                emitter.seq += 1
                frame = encode_frame(emitter.packet.packet_id, payload)
                try:
                    client.sendall(frame)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    if args.verbose:
                        print("[mock] client disconnected")
                    close_client()
                    break

                interval = 1.0 / max(1, emitter.hz)
                while emitter.next_due <= now:
                    emitter.next_due += interval

            time.sleep(0.001)

    except KeyboardInterrupt:
        print("\n[mock] interrupted")
    finally:
        close_client()
        server.close()

    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="OpenOCD-like RTT byte stream mock server (declaration-driven)."
    )
    parser.add_argument(
        "--config",
        default="examples/mock/rat.toml",
        help="path to rat.toml used to resolve rat_gen.toml",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=19021)
    parser.add_argument("--seed", type=int, default=20260208)
    parser.add_argument("--duration", type=float, default=None)
    parser.add_argument(
        "--profile",
        choices=["balanced", "high", "low"],
        default="balanced",
    )
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    return parser


def main() -> int:
    args = build_parser().parse_args()

    if args.self_test:
        run_self_tests()
        return 0

    config_path = Path(args.config)
    if not config_path.is_absolute():
        config_path = (Path.cwd() / config_path).resolve()
    if not config_path.exists():
        print(f"[mock] config not found: {config_path}", file=sys.stderr)
        return 2

    try:
        generated = resolve_generated_toml_path(config_path)
        packets = load_packets(generated)
        schema_bytes = generated.read_bytes()
    except Exception as exc:
        print(f"[mock] {exc}", file=sys.stderr)
        return 3

    return run_server(args, packets, schema_bytes)


if __name__ == "__main__":
    sys.exit(main())

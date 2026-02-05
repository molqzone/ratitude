import argparse
import os
import re
import socket
import subprocess
import sys
import time


def _parse_hex_tokens(text):
    tokens = []
    for line in text.splitlines():
        if ":" not in line:
            continue
        _, rest = line.split(":", 1)
        for token in rest.strip().split():
            if token.startswith("0x"):
                try:
                    tokens.append(int(token, 16))
                except ValueError:
                    continue
            elif re.fullmatch(r"[0-9a-fA-F]{8}", token):
                tokens.append(int(token, 16))
    return tokens


def _parse_byte_tokens(text):
    tokens = []
    for line in text.splitlines():
        if ":" not in line:
            continue
        _, rest = line.split(":", 1)
        for token in rest.strip().split():
            if re.fullmatch(r"[0-9a-fA-F]{2}", token):
                tokens.append(int(token, 16))
    return tokens


class OpenOcdTelnet:
    def __init__(self, host, port, timeout):
        self.host = host
        self.port = port
        self.timeout = timeout
        self.sock = None

    def connect(self):
        self.sock = socket.create_connection((self.host, self.port), timeout=self.timeout)
        self.sock.settimeout(self.timeout)
        self._drain()

    def close(self):
        if self.sock:
            try:
                self.sock.close()
            except OSError:
                pass
            self.sock = None

    def _drain(self):
        if not self.sock:
            return
        self.sock.setblocking(False)
        try:
            while True:
                chunk = self.sock.recv(4096)
                if not chunk:
                    break
        except OSError:
            pass
        finally:
            self.sock.setblocking(True)

    def cmd(self, command):
        if not self.sock:
            raise RuntimeError("openocd telnet not connected")
        self._drain()
        self.sock.sendall((command + "\n").encode("utf-8"))
        return self._read_until_prompt()

    def _read_until_prompt(self):
        data = b""
        deadline = time.time() + self.timeout
        while time.time() < deadline:
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                break
            if not chunk:
                break
            data += chunk
            if b"\n>" in data or data.rstrip().endswith(b">"):
                break
        return data.decode("utf-8", errors="ignore")


def _run_gdb(elf, exprs):
    args = ["arm-none-eabi-gdb", "-q", "-batch", elf]
    args += ["-ex", "set pagination off"]
    args += ["-ex", "target remote :3333"]
    args += ["-ex", "monitor halt"]
    for label, expr in exprs.items():
        args += ["-ex", f"printf \"{label}=0x%lx\\n\", (unsigned long){expr}"]
    args += ["-ex", "monitor resume"]
    args += ["-ex", "detach"]
    args += ["-ex", "quit"]
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result.stdout


def _extract_addresses(output, labels):
    result = {}
    for line in output.splitlines():
        for label in labels:
            if line.startswith(label + "="):
                match = re.findall(r"0x[0-9a-fA-F]+", line)
                if match:
                    result[label] = int(match[-1], 16)
    missing = [label for label in labels if label not in result]
    if missing:
        raise RuntimeError(f"failed to parse symbol addresses: {', '.join(missing)}")
    return result


def _cobs_decode(frame):
    out = bytearray()
    idx = 0
    length = len(frame)
    while idx < length:
        code = frame[idx]
        idx += 1
        if code == 0:
            break
        count = code - 1
        if idx + count > length:
            break
        out.extend(frame[idx:idx + count])
        idx += count
        if code != 0xFF and idx < length:
            out.append(0)
    return bytes(out)


def _read_word(telnet, addr):
    out = telnet.cmd(f"mdw 0x{addr:08x} 1")
    words = _parse_hex_tokens(out)
    if not words:
        raise RuntimeError(f"mdw failed for 0x{addr:08x}: {out.strip()}")
    return words[-1]


def _write_word(telnet, addr, value):
    telnet.cmd(f"mww 0x{addr:08x} 0x{value:08x}")


def _read_bytes(telnet, addr, count, chunk_size):
    data = bytearray()
    remaining = count
    current = addr
    while remaining > 0:
        chunk = min(remaining, chunk_size)
        out = telnet.cmd(f"mdb 0x{current:08x} {chunk}")
        data.extend(_parse_byte_tokens(out))
        remaining -= chunk
        current += chunk
    return bytes(data[:count])


def main():
    parser = argparse.ArgumentParser(description="Poll SEGGER RTT buffer via OpenOCD and decode COBS frames.")
    parser.add_argument("--elf", default="d:/Repos/ratitude/build/stm32f4_rtt/stm32f4_rtt.elf")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=4444)
    parser.add_argument("--interval", type=float, default=0.2)
    parser.add_argument("--channel", type=int, default=0, choices=[0, 1])
    parser.add_argument("--text-id", type=lambda x: int(x, 0), default=0xFF)
    parser.add_argument("--max-bytes", type=int, default=256)
    parser.add_argument("--chunk", type=int, default=64)
    parser.add_argument("--halt", action="store_true")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--max-frames", type=int, default=0)
    parser.add_argument("--reset-run", action="store_true", default=True)
    parser.add_argument("--boot-delay", type=float, default=1.0)
    parser.add_argument("--start-openocd", action="store_true", default=True)
    parser.add_argument("--openocd-interface", default="interface/cmsis-dap.cfg")
    parser.add_argument("--openocd-target", default="target/stm32f4x.cfg")
    parser.add_argument("--openocd-transport", default="swd")
    args = parser.parse_args()

    if not os.path.exists(args.elf):
        print(f"ELF not found: {args.elf}", file=sys.stderr)
        return 2

    openocd_proc = None
    if args.start_openocd:
        openocd_args = [
            "openocd",
            "-f",
            args.openocd_interface,
            "-f",
            args.openocd_target,
            "-c",
            f"transport select {args.openocd_transport}",
        ]
        openocd_proc = subprocess.Popen(openocd_args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1.0)

    telnet = OpenOcdTelnet(args.host, args.port, timeout=3.0)
    try:
        telnet.connect()
    except OSError as exc:
        if openocd_proc:
            openocd_proc.terminate()
        print(f"failed to connect to openocd telnet: {exc}", file=sys.stderr)
        return 3

    try:
        exprs = {
            "WR_ADDR": f"&_SEGGER_RTT.up[{args.channel}].wr",
            "RD_ADDR": f"&_SEGGER_RTT.up[{args.channel}].rd",
            "SIZE_ADDR": f"&_SEGGER_RTT.up[{args.channel}].size",
            "BUF_PTR_ADDR": f"&_SEGGER_RTT.up[{args.channel}].pBuffer",
        }
        gdb_out = _run_gdb(args.elf, exprs)
        addrs = _extract_addresses(gdb_out, exprs.keys())
        wr_addr = addrs["WR_ADDR"]
        rd_addr = addrs["RD_ADDR"]
        size_addr = addrs["SIZE_ADDR"]
        buf_ptr_addr = addrs["BUF_PTR_ADDR"]

        if args.reset_run:
            telnet.cmd("reset run")
            time.sleep(args.boot_delay)

        telnet.cmd("halt")
        buf_ptr = _read_word(telnet, buf_ptr_addr)
        rb_size = _read_word(telnet, size_addr)
        if rb_size == 0:
            time.sleep(args.boot_delay)
            rb_size = _read_word(telnet, size_addr)
        if rb_size == 0:
            raise RuntimeError("ring buffer size is zero; rat_init may not have run yet")
        if not args.halt:
            telnet.cmd("resume")

        print(f"RAT ring buffer: channel={args.channel} size={rb_size} buf=0x{buf_ptr:08x}")
        pending = bytearray()
        frames = 0

        while True:
            if args.halt:
                telnet.cmd("halt")

            wr = _read_word(telnet, wr_addr)
            rd = _read_word(telnet, rd_addr)

            available = (wr - rd) if wr >= rd else (rb_size - rd + wr)
            if available > 0:
                to_read = min(available, args.max_bytes)
                first = min(to_read, rb_size - rd)
                data = bytearray()
                data.extend(_read_bytes(telnet, buf_ptr + rd, first, args.chunk))
                if to_read > first:
                    data.extend(_read_bytes(telnet, buf_ptr, to_read - first, args.chunk))
                rd = (rd + to_read) % rb_size
                _write_word(telnet, rd_addr, rd)

                pending.extend(data)
                while True:
                    try:
                        end = pending.index(0)
                    except ValueError:
                        break
                    frame = bytes(pending[:end])
                    del pending[:end + 1]
                    if not frame:
                        continue
                    decoded = _cobs_decode(frame)
                    if not decoded:
                        continue
                    packet_id = decoded[0]
                    payload = decoded[1:]
                    if packet_id == args.text_id:
                        text = payload.decode("utf-8", errors="replace")
                        print(f"[text] {text}")
                    else:
                        hex_payload = payload.hex()
                        print(f"[pid=0x{packet_id:02x}] {hex_payload}")
                    frames += 1
                    if args.max_frames and frames >= args.max_frames:
                        return 0

            if args.halt:
                telnet.cmd("resume")

            if args.once:
                return 0

            time.sleep(args.interval)
    finally:
        telnet.close()
        if openocd_proc:
            openocd_proc.terminate()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

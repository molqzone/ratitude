#!/usr/bin/env python3
"""Removed mock entrypoint kept only as an explicit hard-stop."""

import sys


def main() -> int:
    sys.stderr.write(
        "openocd_rtt_mock.py is removed and no longer supported.\n"
        "Mock RTT pipeline was decommissioned.\n"
        "Use a real RTT endpoint, then run:\n"
        "  cargo run -p ratsync -- --config <path/to/rat.toml>\n"
        "  cargo run -p ratd -- --config <path/to/rat.toml>\n"
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

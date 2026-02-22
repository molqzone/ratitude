#!/usr/bin/env bash
set -euo pipefail

echo "run_mock_foxglove.sh has been deprecated and is no longer supported."
echo "Mock RTT pipeline was decommissioned."
echo "Use a real RTT endpoint, then run:"
echo "  cargo run -p ratsync -- --config <path/to/rat.toml>"
echo "  cargo run -p ratd -- --config <path/to/rat.toml>"
exit 1

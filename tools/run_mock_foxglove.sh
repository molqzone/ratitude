#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CONFIG_PATH="${1:-examples/mock/rat.toml}"
if [[ "${CONFIG_PATH}" != /* ]]; then
  CONFIG_PATH="${REPO_ROOT}/${CONFIG_PATH}"
fi
MOCK_HOST="${MOCK_HOST:-127.0.0.1}"
MOCK_PORT="${MOCK_PORT:-19021}"

cd "${REPO_ROOT}"

MOCK_PID=""
cleanup() {
  if [[ -n "${MOCK_PID}" ]] && kill -0 "${MOCK_PID}" >/dev/null 2>&1; then
    kill "${MOCK_PID}" >/dev/null 2>&1 || true
    wait "${MOCK_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

python -X utf8 "${REPO_ROOT}/tools/openocd_rtt_mock.py" \
  --config "${CONFIG_PATH}" \
  --host "${MOCK_HOST}" \
  --port "${MOCK_PORT}" \
  --profile balanced &
MOCK_PID="$!"

sleep 0.3

cargo run -p ratd -- --config "${CONFIG_PATH}"

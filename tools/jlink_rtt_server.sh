#!/usr/bin/env bash
set -euo pipefail

device="STM32F407ZG"
iface="SWD"
speed="4000"
rtt_port="19021"
serial=""
ip=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --device)
      device="$2"
      shift 2
      ;;
    --if)
      iface="$2"
      shift 2
      ;;
    --speed)
      speed="$2"
      shift 2
      ;;
    --rtt-port)
      rtt_port="$2"
      shift 2
      ;;
    --serial)
      serial="$2"
      shift 2
      ;;
    --ip)
      ip="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: jlink_rtt_server.sh [options]
  --device <name>      Target device name (default: STM32F407ZG)
  --if <SWD|JTAG>      Target interface (default: SWD)
  --speed <kHz>        Interface speed (default: 4000)
  --rtt-port <port>    RTT Telnet port (default: 19021)
  --serial <sn>        Select J-Link by serial number
  --ip <host>          Select J-Link by IP/host
USAGE
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 2
      ;;
  esac
done

echo "Starting J-Link RTT backend: device=${device}, if=${iface}, speed=${speed}, rtt_port=${rtt_port}"

cmd=(
  JLinkGDBServerCLExe
  -if "$iface"
  -speed "$speed"
  -device "$device"
  -RTTTelnetPort "$rtt_port"
  -silent
  -singlerun
)

if [[ -n "$serial" ]]; then
  cmd+=( -USB "$serial" )
elif [[ -n "$ip" ]]; then
  cmd+=( -IP "$ip" )
fi

exec "${cmd[@]}"

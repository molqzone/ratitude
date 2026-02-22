.PHONY: sync daemon up jlink-rtt mock

sync:
	cargo run -p ratsync -- --config firmware/example/stm32f4_rtt/rat.toml

daemon:
	cargo run -p rttd -- --config firmware/example/stm32f4_rtt/rat.toml

up: daemon

jlink-rtt:
	./tools/jlink_rtt_server.sh --device STM32F407ZG --if SWD --speed 4000 --rtt-port 19021

mock:
	./tools/run_mock_foxglove.sh

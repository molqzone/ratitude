.PHONY: sync server foxglove up jlink-rtt mock-foxglove

sync:
	cargo run -p rttd -- sync --config firmware/example/stm32f4_rtt/rat.toml

server:
	cargo run -p rttd -- server --config firmware/example/stm32f4_rtt/rat.toml

foxglove:
	cargo run -p rttd -- foxglove --config firmware/example/stm32f4_rtt/rat.toml

up: sync foxglove

jlink-rtt:
	./tools/jlink_rtt_server.sh --device STM32F407ZG --if SWD --speed 4000 --rtt-port 19021

mock-foxglove:
	./tools/run_mock_foxglove.sh

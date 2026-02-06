.PHONY: sync server foxglove up

sync:
	cargo run -p rttd -- sync --config firmware/example/stm32f4_rtt/ratitude.toml

server:
	cargo run -p rttd -- server --config firmware/example/stm32f4_rtt/ratitude.toml

foxglove:
	cargo run -p rttd -- foxglove --config firmware/example/stm32f4_rtt/ratitude.toml

up: sync foxglove

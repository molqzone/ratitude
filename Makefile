.PHONY: sync serve up

sync:
	go run tools/rat-gen.go sync --config firmware/example/stm32f4_rtt/ratitude.toml

serve:
	go run ./cmd/rttd

up: sync serve

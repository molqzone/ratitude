package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strconv"
	"time"

	"ratitude/pkg/bridge/foxglove"
	"ratitude/pkg/engine"
	"ratitude/pkg/logger"
	"ratitude/pkg/protocol"
	"ratitude/pkg/transport"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout io.Writer, stderr io.Writer) int {
	if len(args) == 0 {
		return runServer([]string{}, stdout, stderr)
	}

	switch args[0] {
	case "server":
		return runServer(args[1:], stdout, stderr)
	case "foxglove":
		return runFoxglove(args[1:], stdout, stderr)
	case "-h", "--help", "help":
		printUsage(stdout)
		return 0
	default:
		fmt.Fprintln(stderr, "unknown command:", args[0])
		printUsage(stderr)
		return 2
	}
}

func runServer(args []string, stdout io.Writer, stderr io.Writer) int {
	fs := flag.NewFlagSet("server", flag.ContinueOnError)
	fs.SetOutput(stderr)

	addr := fs.String("addr", "127.0.0.1:19021", "TCP address")
	logPath := fs.String("log", "", "JSONL output path (default: stdout)")
	textIDStr := fs.String("text-id", "0xFF", "packet id for text logs")
	reconnect := fs.Duration("reconnect", 1*time.Second, "reconnect interval")
	bufSize := fs.Int("buf", 256, "frame channel buffer size")
	readerBuf := fs.Int("reader-buf", 64*1024, "transport read buffer size")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	textID, err := parseUint8(*textIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --text-id:", err)
		return 2
	}
	protocol.TextPacketID = textID

	var out io.Writer = stdout
	if *logPath != "" {
		file, err := os.Create(*logPath)
		if err != nil {
			fmt.Fprintln(stderr, "failed to open log file:", err)
			return 1
		}
		defer file.Close()
		out = file
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	hub := engine.NewHub()
	go hub.Run(ctx)

	frames := make(chan []byte, *bufSize)
	transport.StartListener(ctx, *addr, frames,
		transport.WithReconnectInterval(*reconnect),
		transport.WithBufferSize(*readerBuf),
	)

	logWriter := logger.NewJSONLWriter(out, textID)
	go logWriter.Consume(ctx, hub.Subscribe())

	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-frames:
				decoded, err := protocol.CobsDecode(frame)
				if err != nil || len(decoded) == 0 {
					continue
				}
				id := decoded[0]
				payload := decoded[1:]
				data, err := protocol.ParsePacket(id, payload)
				if err != nil {
					continue
				}
				hub.Publish(protocol.RatPacket{
					ID:        id,
					Timestamp: time.Now(),
					Payload:   payload,
					Data:      data,
				})
			}
		}
	}()

	<-ctx.Done()
	return 0
}

func runFoxglove(args []string, stdout io.Writer, stderr io.Writer) int {
	fs := flag.NewFlagSet("foxglove", flag.ContinueOnError)
	fs.SetOutput(stderr)

	addr := fs.String("addr", "127.0.0.1:19021", "TCP address")
	wsAddr := fs.String("ws-addr", "127.0.0.1:8765", "Foxglove WebSocket address")
	textIDStr := fs.String("text-id", "0xFF", "packet id for text logs")
	reconnect := fs.Duration("reconnect", 1*time.Second, "reconnect interval")
	bufSize := fs.Int("buf", 256, "frame channel buffer size")
	readerBuf := fs.Int("reader-buf", 64*1024, "transport read buffer size")
	topic := fs.String("topic", foxglove.DefaultConfig().Topic, "Foxglove topic")
	schemaName := fs.String("schema-name", foxglove.DefaultConfig().SchemaName, "Foxglove schema name")
	mock := fs.Bool("mock", false, "generate mock data instead of TCP input")
	mockHz := fs.Int("mock-hz", 50, "mock sample rate (Hz)")
	mockIDStr := fs.String("mock-id", "0x01", "mock packet id")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	textID, err := parseUint8(*textIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --text-id:", err)
		return 2
	}
	protocol.TextPacketID = textID

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	hub := engine.NewHub()
	go hub.Run(ctx)

	cfg := foxglove.DefaultConfig()
	cfg.WSAddr = *wsAddr
	cfg.Topic = *topic
	cfg.SchemaName = *schemaName

	server := foxglove.NewServer(cfg, hub, textID)

	if *mock {
		mockID, err := parseUint8(*mockIDStr)
		if err != nil {
			fmt.Fprintln(stderr, "invalid --mock-id:", err)
			return 2
		}
		go runMockPublisher(ctx, hub, mockID, *mockHz)
	} else {
		frames := make(chan []byte, *bufSize)
		transport.StartListener(ctx, *addr, frames,
			transport.WithReconnectInterval(*reconnect),
			transport.WithBufferSize(*readerBuf),
		)

		go func() {
			for {
				select {
				case <-ctx.Done():
					return
				case frame := <-frames:
					decoded, err := protocol.CobsDecode(frame)
					if err != nil || len(decoded) == 0 {
						continue
					}
					id := decoded[0]
					payload := decoded[1:]
					data, err := protocol.ParsePacket(id, payload)
					if err != nil {
						continue
					}
					hub.Publish(protocol.RatPacket{
						ID:        id,
						Timestamp: time.Now(),
						Payload:   payload,
						Data:      data,
					})
				}
			}
		}()
	}

	if err := server.Run(ctx); err != nil {
		fmt.Fprintln(stderr, "foxglove server error:", err)
		return 1
	}
	return 0
}

func parseUint8(value string) (uint8, error) {
	n, err := strconv.ParseUint(value, 0, 8)
	if err != nil {
		return 0, err
	}
	return uint8(n), nil
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  rttd server [--addr host:port] [--log file.jsonl] [--text-id 0xFF] [--reconnect 1s] [--buf 256] [--reader-buf 65536]")
	fmt.Fprintln(w, "  rttd foxglove [--addr host:port] [--ws-addr host:port] [--text-id 0xFF] [--reconnect 1s] [--buf 256] [--reader-buf 65536] [--topic name] [--schema-name name] [--mock] [--mock-hz 50] [--mock-id 0x01]")
	fmt.Fprintln(w, "")
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  server   start the Ratitude host pipeline")
	fmt.Fprintln(w, "  foxglove start the Foxglove WebSocket bridge")
}

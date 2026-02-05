package main

import (
	"bufio"
	"context"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"ratitude/pkg/engine"
	"ratitude/pkg/logger"
	"ratitude/pkg/protocol"
	"ratitude/pkg/transport"
	"ratitude/pkg/tui"
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
	case "tui":
		return runTUI(args[1:], stdout, stderr)
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

// runTUI starts the plot TUI mode.
// Args:
//
//	args: CLI arguments (without the subcommand).
//	stdout: Standard output writer.
//	stderr: Standard error writer.
//
// Returns:
//
//	Process exit code.
func runTUI(args []string, stdout io.Writer, stderr io.Writer) int {
	fs := flag.NewFlagSet("tui", flag.ContinueOnError)
	fs.SetOutput(stderr)

	addr := fs.String("addr", "", "TCP address (default: 127.0.0.1:19021)")
	idStr := fs.String("id", "0x01", "packet id for plot values")
	reconnect := fs.Duration("reconnect", 1*time.Second, "reconnect interval")
	bufSize := fs.Int("buf", 256, "frame channel buffer size")
	readerBuf := fs.Int("reader-buf", 64*1024, "transport read buffer size")
	stats := fs.Bool("stats", false, "print sample rate statistics to stderr")
	statsInterval := fs.Duration("stats-interval", 1*time.Second, "statistics interval")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	packetID, err := parseUint8(*idStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --id:", err)
		return 2
	}

	addrValue := strings.TrimSpace(*addr)
	if addrValue == "" {
		fmt.Fprint(stdout, "TCP address (default 127.0.0.1:19021): ")
		reader := bufio.NewReader(os.Stdin)
		line, err := reader.ReadString('\n')
		if err != nil && err != io.EOF {
			fmt.Fprintln(stderr, "failed to read address:", err)
			return 1
		}
		addrValue = strings.TrimSpace(line)
		if addrValue == "" {
			addrValue = "127.0.0.1:19021"
		}
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	frames := make(chan []byte, *bufSize)
	transport.StartListener(ctx, addrValue, frames,
		transport.WithReconnectInterval(*reconnect),
		transport.WithBufferSize(*readerBuf),
	)

	samples := make(chan tui.Sample, *bufSize)
	var sentCount int64
	var dropCount int64
	if *stats {
		interval := *statsInterval
		if interval <= 0 {
			interval = time.Second
		}
		go func() {
			ticker := time.NewTicker(interval)
			defer ticker.Stop()
			for {
				select {
				case <-ctx.Done():
					return
				case <-ticker.C:
					sent := atomic.SwapInt64(&sentCount, 0)
					dropped := atomic.SwapInt64(&dropCount, 0)
					rate := float64(sent) / interval.Seconds()
					fmt.Fprintf(stderr, "tui stats: rate=%.1f samples/s dropped=%d\n", rate, dropped)
				}
			}
		}()
	}
	go func() {
		defer close(samples)
		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-frames:
				if len(frame) == 0 {
					continue
				}
				decoded, err := protocol.CobsDecode(frame)
				if err != nil || len(decoded) < 1 {
					continue
				}
				if decoded[0] != packetID {
					continue
				}
				value, ok := tui.DecodeInt32LE(decoded[1:])
				if !ok {
					continue
				}
				select {
				case samples <- tui.Sample{Value: value}:
					if *stats {
						atomic.AddInt64(&sentCount, 1)
					}
				default:
					if *stats {
						atomic.AddInt64(&dropCount, 1)
					}
				}
			}
		}
	}()

	model := tui.NewPlotModel(tui.PlotConfig{}, samples)
	prog := tea.NewProgram(model, tea.WithAltScreen())
	if _, err := prog.Run(); err != nil {
		fmt.Fprintln(stderr, "tui error:", err)
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
	fmt.Fprintln(w, "  rttd tui [--addr host:port] [--id 0x01] [--reconnect 1s] [--buf 256] [--reader-buf 65536] [--stats] [--stats-interval 1s]")
	fmt.Fprintln(w, "")
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  server   start the Ratitude host pipeline")
	fmt.Fprintln(w, "  tui      start the Ratitude plot view")
}

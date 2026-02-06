package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"reflect"
	"strconv"
	"strings"
	"time"

	"ratitude/pkg/bridge/foxglove"
	"ratitude/pkg/config"
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
	ratCfg, configPath, err := loadRuntimeConfig(args)
	if err != nil {
		fmt.Fprintln(stderr, "failed to prepare config:", err)
		return 2
	}
	reconnectDefault := parseDurationDefault(ratCfg.RTTD.Server.Reconnect, time.Second)

	fs := flag.NewFlagSet("server", flag.ContinueOnError)
	fs.SetOutput(stderr)

	_ = fs.String("config", configPath, "ratitude TOML config path")
	addr := fs.String("addr", ratCfg.RTTD.Server.Addr, "TCP address")
	logPath := fs.String("log", "", "JSONL output path (default: stdout)")
	textIDStr := fs.String("text-id", formatUint8Hex(uint8(ratCfg.RTTD.TextID)), "packet id for text logs")
	reconnect := fs.Duration("reconnect", reconnectDefault, "reconnect interval")
	bufSize := fs.Int("buf", ratCfg.RTTD.Server.Buf, "frame channel buffer size")
	readerBuf := fs.Int("reader-buf", ratCfg.RTTD.Server.ReaderBuf, "transport read buffer size")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	textID, err := parseUint8(*textIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --text-id:", err)
		return 2
	}

	protocol.TextPacketID = textID
	if err := registerDynamicPackets(ratCfg.Packets); err != nil {
		fmt.Fprintln(stderr, "invalid packet configuration:", err)
		return 2
	}

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

	go consumeFrames(ctx, frames, hub)

	<-ctx.Done()
	return 0
}

func runFoxglove(args []string, _ io.Writer, stderr io.Writer) int {
	ratCfg, configPath, err := loadRuntimeConfig(args)
	if err != nil {
		fmt.Fprintln(stderr, "failed to prepare config:", err)
		return 2
	}
	reconnectDefault := parseDurationDefault(ratCfg.RTTD.Server.Reconnect, time.Second)

	quatDefault := uint8(ratCfg.RTTD.Foxglove.QuatID)
	if poseID, ok := firstPosePacketID(ratCfg.Packets); ok {
		if !hasFlag(args, "--quat-id") && (ratCfg.RTTD.Foxglove.QuatID == 0 || !hasPacketID(ratCfg.Packets, ratCfg.RTTD.Foxglove.QuatID)) {
			quatDefault = uint8(poseID)
		}
	}

	defaults := foxglove.DefaultConfig()

	fs := flag.NewFlagSet("foxglove", flag.ContinueOnError)
	fs.SetOutput(stderr)

	_ = fs.String("config", configPath, "ratitude TOML config path")
	addr := fs.String("addr", ratCfg.RTTD.Server.Addr, "TCP address")
	wsAddr := fs.String("ws-addr", ratCfg.RTTD.Foxglove.WSAddr, "Foxglove WebSocket address")
	textIDStr := fs.String("text-id", formatUint8Hex(uint8(ratCfg.RTTD.TextID)), "packet id for text logs")
	quatIDStr := fs.String("quat-id", formatUint8Hex(quatDefault), "packet id for quaternion marker packets")
	tempIDStr := fs.String("temp-id", formatUint8Hex(uint8(ratCfg.RTTD.Foxglove.TempID)), "packet id for temperature gauge packets")
	reconnect := fs.Duration("reconnect", reconnectDefault, "reconnect interval")
	bufSize := fs.Int("buf", ratCfg.RTTD.Server.Buf, "frame channel buffer size")
	readerBuf := fs.Int("reader-buf", ratCfg.RTTD.Server.ReaderBuf, "transport read buffer size")
	topic := fs.String("topic", ratCfg.RTTD.Foxglove.Topic, "Foxglove topic")
	schemaName := fs.String("schema-name", ratCfg.RTTD.Foxglove.SchemaName, "Foxglove schema name")
	markerTopic := fs.String("marker-topic", ratCfg.RTTD.Foxglove.MarkerTopic, "Foxglove marker topic")
	parentFrameID := fs.String("parent-frame", ratCfg.RTTD.Foxglove.ParentFrame, "transform parent frame id")
	frameID := fs.String("frame-id", ratCfg.RTTD.Foxglove.FrameID, "marker frame id")
	imagePath := fs.String("image-path", ratCfg.RTTD.Foxglove.ImagePath, "path to compressed image file for /camera/image/compressed")
	imageFrameID := fs.String("image-frame", ratCfg.RTTD.Foxglove.ImageFrame, "frame id for compressed image stream")
	imageFormat := fs.String("image-format", ratCfg.RTTD.Foxglove.ImageFormat, "compressed image format")
	logTopic := fs.String("log-topic", ratCfg.RTTD.Foxglove.LogTopic, "Foxglove Log Panel topic")
	logName := fs.String("log-name", ratCfg.RTTD.Foxglove.LogName, "source name used in foxglove.Log messages")
	mock := fs.Bool("mock", false, "generate mock IMU quaternion packets instead of TCP input")
	mockHz := fs.Int("mock-hz", 50, "mock sample rate (Hz)")
	mockIDStr := fs.String("mock-id", formatUint8Hex(uint8(ratCfg.RTTD.Foxglove.QuatID)), "mock packet id")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	textID, err := parseUint8(*textIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --text-id:", err)
		return 2
	}
	quatID, err := parseUint8(*quatIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --quat-id:", err)
		return 2
	}
	tempID, err := parseUint8(*tempIDStr)
	if err != nil {
		fmt.Fprintln(stderr, "invalid --temp-id:", err)
		return 2
	}

	protocol.TextPacketID = textID
	protocol.Register(quatID, reflect.TypeOf(protocol.QuatPacket{}))
	protocol.Register(tempID, reflect.TypeOf(protocol.TemperaturePacket{}))

	if err := registerDynamicPackets(ratCfg.Packets); err != nil {
		fmt.Fprintln(stderr, "invalid packet configuration:", err)
		return 2
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	hub := engine.NewHub()
	go hub.Run(ctx)

	cfg := defaults
	cfg.WSAddr = *wsAddr
	cfg.Topic = *topic
	cfg.SchemaName = *schemaName
	cfg.MarkerTopic = *markerTopic
	cfg.ParentFrameID = *parentFrameID
	cfg.FrameID = *frameID
	cfg.ImagePath = *imagePath
	cfg.ImageFrameID = *imageFrameID
	cfg.ImageFormat = *imageFormat
	cfg.LogTopic = *logTopic
	cfg.LogName = *logName

	server := foxglove.NewServer(cfg, hub, textID, quatID)

	if *mock {
		mockID, err := parseUint8(*mockIDStr)
		if err != nil {
			fmt.Fprintln(stderr, "invalid --mock-id:", err)
			return 2
		}
		go runMockPublisher(ctx, hub, mockID, textID, tempID, *mockHz)
	} else {
		frames := make(chan []byte, *bufSize)
		transport.StartListener(ctx, *addr, frames,
			transport.WithReconnectInterval(*reconnect),
			transport.WithBufferSize(*readerBuf),
		)
		go consumeFrames(ctx, frames, hub)
	}

	if err := server.Run(ctx); err != nil {
		fmt.Fprintln(stderr, "foxglove server error:", err)
		return 1
	}
	return 0
}

func registerDynamicPackets(packets []config.PacketDef) error {
	protocol.ClearDynamicRegistry()
	for _, pkt := range packets {
		if pkt.ID > 0xFF {
			return fmt.Errorf("packet id out of range: 0x%x", pkt.ID)
		}
		def := protocol.DynamicPacketDef{
			ID:         uint8(pkt.ID),
			StructName: pkt.StructName,
			Packed:     pkt.Packed,
			ByteSize:   pkt.ByteSize,
			Fields:     make([]protocol.DynamicFieldDef, 0, len(pkt.Fields)),
		}
		for _, field := range pkt.Fields {
			def.Fields = append(def.Fields, protocol.DynamicFieldDef{
				Name:   field.Name,
				CType:  field.CType,
				Offset: field.Offset,
				Size:   field.Size,
			})
		}
		if err := protocol.RegisterDynamic(uint8(pkt.ID), def); err != nil {
			return fmt.Errorf("register packet 0x%02x (%s): %w", pkt.ID, pkt.StructName, err)
		}
	}
	return nil
}

func consumeFrames(ctx context.Context, frames <-chan []byte, hub *engine.Hub) {
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
}

func parseUint8(value string) (uint8, error) {
	n, err := strconv.ParseUint(value, 0, 8)
	if err != nil {
		return 0, err
	}
	return uint8(n), nil
}

func parseDurationDefault(raw string, fallback time.Duration) time.Duration {
	if strings.TrimSpace(raw) == "" {
		return fallback
	}
	d, err := time.ParseDuration(raw)
	if err != nil {
		return fallback
	}
	return d
}

func formatUint8Hex(value uint8) string {
	return fmt.Sprintf("0x%02X", value)
}

func extractConfigPath(args []string, fallback string) (string, error) {
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if arg == "--config" {
			if i+1 >= len(args) {
				return "", fmt.Errorf("missing value")
			}
			return args[i+1], nil
		}
		if strings.HasPrefix(arg, "--config=") {
			value := strings.TrimPrefix(arg, "--config=")
			if value == "" {
				return "", fmt.Errorf("missing value")
			}
			return value, nil
		}
	}
	return fallback, nil
}

func loadRuntimeConfig(args []string) (config.RatitudeConfig, string, error) {
	configPath, err := extractConfigPath(args, config.DefaultConfigPath)
	if err != nil {
		return config.RatitudeConfig{}, "", err
	}

	if !hasFlag(args, "--help") {
		if _, _, err := config.SyncPackets(configPath, ""); err != nil {
			return config.RatitudeConfig{}, configPath, err
		}
	}

	cfg, _, err := config.LoadOrDefault(configPath)
	if err != nil {
		return config.RatitudeConfig{}, configPath, err
	}
	return cfg, configPath, nil
}

func hasFlag(args []string, name string) bool {
	for _, arg := range args {
		if arg == name {
			return true
		}
		if strings.HasPrefix(arg, name+"=") {
			return true
		}
	}
	if name == "--help" {
		for _, arg := range args {
			if arg == "-h" {
				return true
			}
		}
	}
	return false
}

func firstPosePacketID(packets []config.PacketDef) (uint16, bool) {
	for _, pkt := range packets {
		if strings.EqualFold(strings.TrimSpace(pkt.Type), "pose_3d") {
			return pkt.ID, true
		}
	}
	return 0, false
}

func hasPacketID(packets []config.PacketDef, id uint16) bool {
	for _, pkt := range packets {
		if pkt.ID == id {
			return true
		}
	}
	return false
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  rttd server [--config path] [--addr host:port] [--log file.jsonl] [--text-id 0xFF] [--reconnect 1s] [--buf 256] [--reader-buf 65536]")
	fmt.Fprintln(w, "  rttd foxglove [--config path] [--addr host:port] [--ws-addr host:port] [--text-id 0xFF] [--quat-id 0x10] [--temp-id 0x20] [--reconnect 1s] [--buf 256] [--reader-buf 65536] [--topic name] [--schema-name name] [--marker-topic /visualization_marker] [--parent-frame world] [--frame-id base_link] [--image-path path] [--image-frame camera] [--image-format jpeg] [--log-topic /ratitude/log] [--log-name ratitude] [--mock] [--mock-hz 50] [--mock-id 0x10]")
	fmt.Fprintln(w, "")
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  server   start the Ratitude host pipeline")
	fmt.Fprintln(w, "  foxglove start the Foxglove WebSocket bridge")
}

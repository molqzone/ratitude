package logger_test

import (
	"bytes"
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"

	"ratitude/pkg/logger"
	"ratitude/pkg/protocol"
)

func TestJSONLWriter(t *testing.T) {
	var buf bytes.Buffer
	writer := logger.NewJSONLWriter(&buf, 0xFF)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	ch := make(chan protocol.RatPacket, 1)
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		writer.Consume(ctx, ch)
	}()

	ts := time.Date(2026, 2, 5, 16, 0, 0, 0, time.UTC)
	ch <- protocol.RatPacket{
		ID:        0xFF,
		Timestamp: ts,
		Payload:   []byte("hi"),
		Data:      "hi",
	}
	close(ch)
	wg.Wait()

	line := strings.TrimSpace(buf.String())
	if line == "" {
		t.Fatalf("expected output line")
	}

	var rec map[string]any
	if err := json.Unmarshal([]byte(line), &rec); err != nil {
		t.Fatalf("json unmarshal failed: %v", err)
	}

	if rec["id"] != "0xff" {
		t.Fatalf("unexpected id: %v", rec["id"])
	}
	if rec["payload_hex"] != "6869" {
		t.Fatalf("unexpected payload_hex: %v", rec["payload_hex"])
	}
	if rec["text"] != "hi" {
		t.Fatalf("unexpected text: %v", rec["text"])
	}
	if rec["data"] != "hi" {
		t.Fatalf("unexpected data: %v", rec["data"])
	}
	tsValue, ok := rec["ts"].(string)
	if !ok || tsValue == "" {
		t.Fatalf("missing ts field")
	}
	if _, err := time.Parse(time.RFC3339Nano, tsValue); err != nil {
		t.Fatalf("invalid ts format: %v", err)
	}
}

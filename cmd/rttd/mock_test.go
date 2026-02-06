package main

import (
	"encoding/binary"
	"testing"
	"time"
)

func TestMockPayloadLE(t *testing.T) {
	value := int32(-1234)
	payload := mockPayload(value)
	if len(payload) != 4 {
		t.Fatalf("unexpected payload size: %d", len(payload))
	}
	got := int32(binary.LittleEndian.Uint32(payload))
	if got != value {
		t.Fatalf("unexpected payload value: %d", got)
	}
}

func TestNewMockPacket(t *testing.T) {
	ts := time.Unix(0, 42)
	pkt := newMockPacket(0x01, 7, 3, ts)
	if pkt.ID != 0x01 {
		t.Fatalf("unexpected packet id: %d", pkt.ID)
	}
	if pkt.Timestamp != ts {
		t.Fatalf("unexpected timestamp: %v", pkt.Timestamp)
	}
	data, ok := pkt.Data.(map[string]any)
	if !ok {
		t.Fatalf("unexpected data type: %T", pkt.Data)
	}
	if data["value"].(int32) != 7 {
		t.Fatalf("unexpected value: %v", data["value"])
	}
	if data["seq"].(int64) != 3 {
		t.Fatalf("unexpected seq: %v", data["seq"])
	}
}

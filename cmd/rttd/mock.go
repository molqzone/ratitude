package main

import (
	"context"
	"encoding/binary"
	"time"

	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

func runMockPublisher(ctx context.Context, hub *engine.Hub, id uint8, hz int) {
	if hz <= 0 {
		hz = 50
	}
	interval := time.Second / time.Duration(hz)
	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	value := int32(-1000)
	step := int32(20)
	dir := int32(1)
	var seq int64

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			value += step * dir
			if value >= 1000 {
				value = 1000
				dir = -1
			} else if value <= -1000 {
				value = -1000
				dir = 1
			}
			pkt := newMockPacket(id, value, seq, time.Now())
			hub.Publish(pkt)
			seq++
		}
	}
}

func newMockPacket(id uint8, value int32, seq int64, ts time.Time) protocol.RatPacket {
	payload := mockPayload(value)
	data := map[string]any{
		"value": value,
		"seq":   seq,
	}
	return protocol.RatPacket{
		ID:        id,
		Timestamp: ts,
		Payload:   payload,
		Data:      data,
	}
}

func mockPayload(value int32) []byte {
	buf := make([]byte, 4)
	binary.LittleEndian.PutUint32(buf, uint32(value))
	return buf
}

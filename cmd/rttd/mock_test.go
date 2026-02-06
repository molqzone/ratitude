package main

import (
	"encoding/binary"
	"math"
	"testing"
	"time"

	"ratitude/pkg/protocol"
)

func TestMockPayloadLE(t *testing.T) {
	quat := protocol.QuatPacket{W: 1.0, X: -0.5, Y: 0.25, Z: -0.125}
	payload := mockPayload(quat)
	if len(payload) != 16 {
		t.Fatalf("unexpected payload size: %d", len(payload))
	}

	got := protocol.QuatPacket{
		W: math.Float32frombits(binary.LittleEndian.Uint32(payload[0:4])),
		X: math.Float32frombits(binary.LittleEndian.Uint32(payload[4:8])),
		Y: math.Float32frombits(binary.LittleEndian.Uint32(payload[8:12])),
		Z: math.Float32frombits(binary.LittleEndian.Uint32(payload[12:16])),
	}
	if got != quat {
		t.Fatalf("unexpected quat payload decode: got=%+v want=%+v", got, quat)
	}
}

func TestNewMockPacket(t *testing.T) {
	ts := time.Unix(0, 42)
	quat := protocol.QuatPacket{W: 1, Z: 0}
	pkt := newMockPacket(0x10, quat, 3, ts)
	if pkt.ID != 0x10 {
		t.Fatalf("unexpected packet id: %d", pkt.ID)
	}
	if pkt.Timestamp != ts {
		t.Fatalf("unexpected timestamp: %v", pkt.Timestamp)
	}
	if len(pkt.Payload) != 16 {
		t.Fatalf("unexpected payload size: %d", len(pkt.Payload))
	}
	parsed, ok := pkt.Data.(protocol.QuatPacket)
	if !ok {
		t.Fatalf("unexpected data type: %T", pkt.Data)
	}
	if parsed.W != 1 || parsed.X != 0 || parsed.Y != 0 || parsed.Z != 0 {
		t.Fatalf("unexpected quaternion: %+v", parsed)
	}
}

func TestMockEulerAnglesTriAxis(t *testing.T) {
	r0, p0, y0 := mockEulerAngles(0)
	r1, p1, y1 := mockEulerAngles(1.0)
	if r0 == r1 || p0 == p1 || y0 == y1 {
		t.Fatalf("expected all axes to vary over time: r=%f/%f p=%f/%f y=%f/%f", r0, r1, p0, p1, y0, y1)
	}
	if math.Abs(r1) < 1e-4 || math.Abs(p1) < 1e-4 || math.Abs(y1) < 1e-4 {
		t.Fatalf("expected non-trivial tri-axis motion: r=%f p=%f y=%f", r1, p1, y1)
	}
}

func TestMockQuaternionNormalizedAndVarying(t *testing.T) {
	q0 := mockQuaternion(0)
	q1 := mockQuaternion(1.0)
	if q0 == q1 {
		t.Fatalf("expected quaternion to change over time")
	}
	norm := math.Sqrt(float64(q1.W*q1.W + q1.X*q1.X + q1.Y*q1.Y + q1.Z*q1.Z))
	if math.Abs(norm-1.0) > 1e-4 {
		t.Fatalf("expected normalized quaternion, got norm=%f", norm)
	}
	if math.Abs(float64(q1.X)) < 1e-4 || math.Abs(float64(q1.Y)) < 1e-4 || math.Abs(float64(q1.Z)) < 1e-4 {
		t.Fatalf("expected xyz components to all participate, got %+v", q1)
	}
}

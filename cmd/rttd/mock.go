package main

import (
	"context"
	"encoding/binary"
	"math"
	"time"

	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

const (
	mockRollAmplitudeRad  = 35.0 * math.Pi / 180.0
	mockPitchAmplitudeRad = 25.0 * math.Pi / 180.0
	mockYawAmplitudeRad   = 40.0 * math.Pi / 180.0

	mockRollFreqHz  = 0.23
	mockPitchFreqHz = 0.31
	mockYawFreqHz   = 0.17

	mockRollPhaseRad  = 0.0
	mockPitchPhaseRad = math.Pi / 3.0
	mockYawPhaseRad   = 2.0 * math.Pi / 3.0
)

func runMockPublisher(ctx context.Context, hub *engine.Hub, id uint8, hz int) {
	if hz <= 0 {
		hz = 50
	}
	interval := time.Second / time.Duration(hz)
	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	start := time.Now()
	var seq int64
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			t := time.Since(start).Seconds()
			quat := mockQuaternion(t)
			pkt := newMockPacket(id, quat, seq, time.Now())
			hub.Publish(pkt)
			seq++
		}
	}
}

func newMockPacket(id uint8, quat protocol.QuatPacket, seq int64, ts time.Time) protocol.RatPacket {
	_ = seq
	payload := mockPayload(quat)
	return protocol.RatPacket{
		ID:        id,
		Timestamp: ts,
		Payload:   payload,
		Data:      quat,
	}
}

func mockEulerAngles(t float64) (roll float64, pitch float64, yaw float64) {
	roll = mockRollAmplitudeRad * math.Sin(2.0*math.Pi*mockRollFreqHz*t+mockRollPhaseRad)
	pitch = mockPitchAmplitudeRad * math.Sin(2.0*math.Pi*mockPitchFreqHz*t+mockPitchPhaseRad)
	yaw = mockYawAmplitudeRad * math.Sin(2.0*math.Pi*mockYawFreqHz*t+mockYawPhaseRad)
	return
}

func mockQuaternion(t float64) protocol.QuatPacket {
	roll, pitch, yaw := mockEulerAngles(t)
	cr := math.Cos(roll * 0.5)
	sr := math.Sin(roll * 0.5)
	cp := math.Cos(pitch * 0.5)
	sp := math.Sin(pitch * 0.5)
	cy := math.Cos(yaw * 0.5)
	sy := math.Sin(yaw * 0.5)

	// ZYX intrinsic rotation (yaw -> pitch -> roll).
	w := cr*cp*cy + sr*sp*sy
	x := sr*cp*cy - cr*sp*sy
	y := cr*sp*cy + sr*cp*sy
	z := cr*cp*sy - sr*sp*cy

	norm := math.Sqrt(w*w + x*x + y*y + z*z)
	if norm == 0 {
		return protocol.QuatPacket{W: 1}
	}
	inv := 1.0 / norm
	return protocol.QuatPacket{
		W: float32(w * inv),
		X: float32(x * inv),
		Y: float32(y * inv),
		Z: float32(z * inv),
	}
}

func mockPayload(quat protocol.QuatPacket) []byte {
	buf := make([]byte, 16)
	binary.LittleEndian.PutUint32(buf[0:4], math.Float32bits(quat.W))
	binary.LittleEndian.PutUint32(buf[4:8], math.Float32bits(quat.X))
	binary.LittleEndian.PutUint32(buf[8:12], math.Float32bits(quat.Y))
	binary.LittleEndian.PutUint32(buf[12:16], math.Float32bits(quat.Z))
	return buf
}

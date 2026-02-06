package protocol

import "time"

// RatPacket is the normalized payload flowing through the pipeline.
type RatPacket struct {
	ID        uint8
	Timestamp time.Time
	Payload   []byte
	Data      any
}

// Parser decodes a payload into a concrete Go value.
type Parser interface {
	Parse(id uint8, payload []byte) (any, error)
}

// QuatPacket mirrors MCU payload layout: struct { float w, x, y, z; }.
type QuatPacket struct {
	W float32 `json:"w"`
	X float32 `json:"x"`
	Y float32 `json:"y"`
	Z float32 `json:"z"`
}

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

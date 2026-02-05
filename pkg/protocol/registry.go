package protocol

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
	"sync"
)

// TextPacketID defines which ID should be parsed as text payload.
var TextPacketID uint8 = 0xFF

var (
	registryMu   sync.RWMutex
	typeRegistry = map[uint8]reflect.Type{}
)

// RawPacket preserves unknown payloads for downstream consumers.
type RawPacket struct {
	ID      uint8
	Payload []byte
}

func (rp RawPacket) MarshalJSON() ([]byte, error) {
	type rawPacketJSON struct {
		ID         string `json:"id"`
		PayloadHex string `json:"payload_hex"`
	}
	return json.Marshal(rawPacketJSON{
		ID:         fmt.Sprintf("0x%02x", rp.ID),
		PayloadHex: hex.EncodeToString(rp.Payload),
	})
}

// Register maps a packet ID to a struct type.
func Register(id uint8, t reflect.Type) {
	if t.Kind() == reflect.Pointer {
		t = t.Elem()
	}
	registryMu.Lock()
	typeRegistry[id] = t
	registryMu.Unlock()
}

// ParseText converts a text payload into a Go string.
func ParseText(payload []byte) string {
	if idx := bytes.IndexByte(payload, 0x00); idx >= 0 {
		payload = payload[:idx]
	}
	return strings.TrimRight(string(payload), "\x00")
}

// ParsePacket decodes a payload into a registered Go type.
func ParsePacket(id uint8, payload []byte) (any, error) {
	if id == TextPacketID {
		return ParseText(payload), nil
	}

	registryMu.RLock()
	t, ok := typeRegistry[id]
	registryMu.RUnlock()
	if !ok {
		return RawPacket{ID: id, Payload: append([]byte(nil), payload...)}, nil
	}

	val := reflect.New(t).Interface()
	size := binary.Size(reflect.ValueOf(val).Elem().Interface())
	if size < 0 {
		return nil, fmt.Errorf("unsupported type size for id 0x%02x", id)
	}
	if len(payload) != size {
		return nil, fmt.Errorf("payload size %d does not match type size %d for id 0x%02x", len(payload), size, id)
	}

	reader := bytes.NewReader(payload)
	if err := binary.Read(reader, binary.LittleEndian, val); err != nil {
		return nil, err
	}

	return reflect.ValueOf(val).Elem().Interface(), nil
}

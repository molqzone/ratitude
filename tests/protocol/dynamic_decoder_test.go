package protocol_test

import (
	"encoding/binary"
	"reflect"
	"testing"

	"ratitude/pkg/protocol"
)

func TestDynamicDecodeMap(t *testing.T) {
	protocol.ClearDynamicRegistry()

	err := protocol.RegisterDynamic(0x42, protocol.DynamicPacketDef{
		ID:       0x42,
		ByteSize: 8,
		Fields: []protocol.DynamicFieldDef{
			{Name: "value", CType: "int32_t", Offset: 0, Size: 4},
			{Name: "tick_ms", CType: "uint32_t", Offset: 4, Size: 4},
		},
	})
	if err != nil {
		t.Fatalf("register dynamic: %v", err)
	}

	payload := make([]byte, 8)
	value := int32(-12)
	binary.LittleEndian.PutUint32(payload[0:4], uint32(value))
	binary.LittleEndian.PutUint32(payload[4:8], 123)

	decoded, err := protocol.ParsePacket(0x42, payload)
	if err != nil {
		t.Fatalf("parse packet: %v", err)
	}

	m, ok := decoded.(map[string]any)
	if !ok {
		t.Fatalf("expected map decode, got %T", decoded)
	}
	if got := m["value"]; !reflect.DeepEqual(got, int32(-12)) {
		t.Fatalf("unexpected value: %#v", got)
	}
	if got := m["tick_ms"]; !reflect.DeepEqual(got, uint32(123)) {
		t.Fatalf("unexpected tick_ms: %#v", got)
	}
}

func TestDynamicDecodeSizeMismatch(t *testing.T) {
	protocol.ClearDynamicRegistry()

	err := protocol.RegisterDynamic(0x33, protocol.DynamicPacketDef{
		ID:       0x33,
		ByteSize: 4,
		Fields: []protocol.DynamicFieldDef{
			{Name: "v", CType: "uint32_t", Offset: 0, Size: 4},
		},
	})
	if err != nil {
		t.Fatalf("register dynamic: %v", err)
	}

	if _, err := protocol.ParsePacket(0x33, []byte{0x01}); err == nil {
		t.Fatalf("expected size mismatch error")
	}
}

func TestTextTakesPrecedenceOverDynamic(t *testing.T) {
	protocol.ClearDynamicRegistry()
	oldTextID := protocol.TextPacketID
	protocol.TextPacketID = 0x55
	defer func() { protocol.TextPacketID = oldTextID }()

	err := protocol.RegisterDynamic(0x55, protocol.DynamicPacketDef{
		ID:       0x55,
		ByteSize: 4,
		Fields: []protocol.DynamicFieldDef{
			{Name: "v", CType: "uint32_t", Offset: 0, Size: 4},
		},
	})
	if err != nil {
		t.Fatalf("register dynamic: %v", err)
	}

	decoded, err := protocol.ParsePacket(0x55, []byte{'o', 'k', 0x00, 0x00})
	if err != nil {
		t.Fatalf("parse packet: %v", err)
	}
	text, ok := decoded.(string)
	if !ok {
		t.Fatalf("expected text string, got %T", decoded)
	}
	if text != "ok" {
		t.Fatalf("unexpected text: %q", text)
	}
}

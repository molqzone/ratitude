package protocol_test

import (
	"reflect"
	"testing"

	"ratitude/pkg/protocol"
)

type samplePacket struct {
	A uint16
	B uint8
}

func TestParsePacketSizeMismatch(t *testing.T) {
	protocol.ClearDynamicRegistry()
	protocol.Register(0x01, reflect.TypeOf(samplePacket{}))

	_, err := protocol.ParsePacket(0x01, []byte{0xAA})
	if err == nil {
		t.Fatalf("expected size mismatch error")
	}
}

func TestParsePacketUnknown(t *testing.T) {
	protocol.ClearDynamicRegistry()
	data, err := protocol.ParsePacket(0x7E, []byte{0x01, 0x02})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	raw, ok := data.(protocol.RawPacket)
	if !ok {
		t.Fatalf("expected RawPacket, got %T", data)
	}
	if raw.ID != 0x7E {
		t.Fatalf("unexpected id: 0x%02x", raw.ID)
	}
	if len(raw.Payload) != 2 || raw.Payload[0] != 0x01 || raw.Payload[1] != 0x02 {
		t.Fatalf("unexpected payload: %v", raw.Payload)
	}
}

func TestParsePacketText(t *testing.T) {
	protocol.ClearDynamicRegistry()
	old := protocol.TextPacketID
	protocol.TextPacketID = 0xEE
	defer func() {
		protocol.TextPacketID = old
	}()

	data, err := protocol.ParsePacket(0xEE, []byte{'h', 'i', 0x00})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	text, ok := data.(string)
	if !ok {
		t.Fatalf("expected string, got %T", data)
	}
	if text != "hi" {
		t.Fatalf("unexpected text: %q", text)
	}
}

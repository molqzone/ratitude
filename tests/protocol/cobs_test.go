package protocol_test

import (
	"testing"

	"ratitude/pkg/protocol"
)

func TestCobsDecodeSimple(t *testing.T) {
	decoded, err := protocol.CobsDecode([]byte{0x03, 0x11, 0x22})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(decoded) != 2 || decoded[0] != 0x11 || decoded[1] != 0x22 {
		t.Fatalf("unexpected decode result: %v", decoded)
	}
}

func TestCobsDecodeWithZero(t *testing.T) {
	frame := []byte{0x02, 0x11, 0x02, 0x22}
	decoded, err := protocol.CobsDecode(frame)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := []byte{0x11, 0x00, 0x22}
	if len(decoded) != len(want) {
		t.Fatalf("unexpected decode length: %d", len(decoded))
	}
	for i := range want {
		if decoded[i] != want[i] {
			t.Fatalf("byte %d mismatch: got 0x%02x want 0x%02x", i, decoded[i], want[i])
		}
	}
}

func TestCobsDecodeInvalid(t *testing.T) {
	if _, err := protocol.CobsDecode([]byte{0x00, 0x01}); err == nil {
		t.Fatalf("expected error for invalid code 0x00")
	}
}

package transport_test

import (
	"context"
	"net"
	"testing"
	"time"

	"ratitude/pkg/transport"
)

func TestListenerDeframe(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen failed: %v", err)
	}
	defer ln.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	out := make(chan []byte, 4)
	transport.StartListener(ctx, ln.Addr().String(), out,
		transport.WithReconnectInterval(10*time.Millisecond),
		transport.WithDialTimeout(200*time.Millisecond),
		transport.WithBufferSize(128),
	)

	conn, err := ln.Accept()
	if err != nil {
		t.Fatalf("accept failed: %v", err)
	}
	defer conn.Close()

	if _, err := conn.Write([]byte{0x11}); err != nil {
		t.Fatalf("write failed: %v", err)
	}
	time.Sleep(10 * time.Millisecond)
	if _, err := conn.Write([]byte{0x22, 0x00, 0x33, 0x00}); err != nil {
		t.Fatalf("write failed: %v", err)
	}

	first := readFrame(t, out)
	second := readFrame(t, out)

	if len(first) != 2 || first[0] != 0x11 || first[1] != 0x22 {
		t.Fatalf("unexpected first frame: %v", first)
	}
	if len(second) != 1 || second[0] != 0x33 {
		t.Fatalf("unexpected second frame: %v", second)
	}
}

func readFrame(t *testing.T, ch <-chan []byte) []byte {
	t.Helper()
	select {
	case frame := <-ch:
		return frame
	case <-time.After(1 * time.Second):
		t.Fatalf("timeout waiting for frame")
		return nil
	}
}

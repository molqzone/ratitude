package engine_test

import (
	"context"
	"testing"
	"time"

	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

func TestHubDoesNotBlockOnSlowConsumer(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	hub := engine.NewHub(engine.WithBroadcastBuffer(1), engine.WithClientBuffer(1))
	go hub.Run(ctx)

	fast := hub.SubscribeWithBuffer(128)
	slow := hub.SubscribeWithBuffer(1)

	done := make(chan struct{})
	go func() {
		for i := 0; i < 50; i++ {
			hub.Publish(protocol.RatPacket{ID: uint8(i)})
		}
		close(done)
	}()

	select {
	case <-done:
	case <-time.After(1 * time.Second):
		t.Fatalf("publish blocked on slow consumer")
	}

	received := 0
	timeout := time.After(1 * time.Second)
	for received < 50 {
		select {
		case <-fast:
			received++
		case <-timeout:
			t.Fatalf("fast consumer timeout after %d packets", received)
		}
	}

	count := 0
	for {
		select {
		case <-slow:
			count++
		default:
			if count > 1 {
				t.Fatalf("slow consumer received %d packets, expected at most 1", count)
			}
			return
		}
	}
}

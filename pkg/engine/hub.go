package engine

import (
	"context"

	"ratitude/pkg/protocol"
)

type Hub struct {
	broadcast  chan protocol.RatPacket
	register   chan chan protocol.RatPacket
	unregister chan chan protocol.RatPacket
	clients    map[chan protocol.RatPacket]struct{}
	clientBuf  int
}

type Option func(*Hub)

func WithBroadcastBuffer(size int) Option {
	return func(h *Hub) {
		if size > 0 {
			h.broadcast = make(chan protocol.RatPacket, size)
		}
	}
}

func WithClientBuffer(size int) Option {
	return func(h *Hub) {
		if size > 0 {
			h.clientBuf = size
		}
	}
}

func NewHub(opts ...Option) *Hub {
	h := &Hub{
		broadcast:  make(chan protocol.RatPacket, 256),
		register:   make(chan chan protocol.RatPacket),
		unregister: make(chan chan protocol.RatPacket),
		clients:    make(map[chan protocol.RatPacket]struct{}),
		clientBuf:  100,
	}
	for _, opt := range opts {
		opt(h)
	}
	return h
}

func (h *Hub) Run(ctx context.Context) {
	for {
		select {
		case <-ctx.Done():
			for ch := range h.clients {
				close(ch)
			}
			return
		case ch := <-h.register:
			h.clients[ch] = struct{}{}
		case ch := <-h.unregister:
			if _, ok := h.clients[ch]; ok {
				delete(h.clients, ch)
				close(ch)
			}
		case packet := <-h.broadcast:
			for ch := range h.clients {
				select {
				case ch <- packet:
				default:
				}
			}
		}
	}
}

func (h *Hub) Subscribe() chan protocol.RatPacket {
	return h.SubscribeWithBuffer(h.clientBuf)
}

func (h *Hub) SubscribeWithBuffer(size int) chan protocol.RatPacket {
	if size <= 0 {
		size = h.clientBuf
	}
	ch := make(chan protocol.RatPacket, size)
	h.register <- ch
	return ch
}

func (h *Hub) Unsubscribe(ch chan protocol.RatPacket) {
	h.unregister <- ch
}

func (h *Hub) Publish(packet protocol.RatPacket) {
	h.broadcast <- packet
}

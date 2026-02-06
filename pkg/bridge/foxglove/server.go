package foxglove

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

type FoxglovePacket struct {
	ID         string `json:"id"`
	TS         string `json:"ts,omitempty"`
	PayloadHex string `json:"payload_hex"`
	Data       any    `json:"data,omitempty"`
	Text       string `json:"text,omitempty"`
}

type Server struct {
	cfg     Config
	hub     *engine.Hub
	textID  uint8
	clients map[*client]struct{}
	mu      sync.RWMutex
	ctx     context.Context
}

type client struct {
	conn *websocket.Conn
	send chan []byte
	subs map[uint32]struct{}
	mu   sync.RWMutex
	once sync.Once
}

func NewServer(cfg Config, hub *engine.Hub, textID uint8) *Server {
	if cfg.WSAddr == "" {
		cfg.WSAddr = DefaultConfig().WSAddr
	}
	if cfg.Name == "" {
		cfg.Name = DefaultConfig().Name
	}
	if cfg.Topic == "" {
		cfg.Topic = DefaultConfig().Topic
	}
	if cfg.ChannelID == 0 {
		cfg.ChannelID = DefaultConfig().ChannelID
	}
	if cfg.SchemaName == "" {
		cfg.SchemaName = DefaultConfig().SchemaName
	}
	if cfg.SchemaEncoding == "" {
		cfg.SchemaEncoding = DefaultConfig().SchemaEncoding
	}
	if cfg.Schema == "" {
		cfg.Schema = DefaultConfig().Schema
	}
	if cfg.Encoding == "" {
		cfg.Encoding = DefaultConfig().Encoding
	}
	if cfg.SendBuf <= 0 {
		cfg.SendBuf = DefaultConfig().SendBuf
	}
	return &Server{
		cfg:     cfg,
		hub:     hub,
		textID:  textID,
		clients: make(map[*client]struct{}),
	}
}

func (s *Server) Run(ctx context.Context) error {
	s.ctx = ctx

	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleWS)

	httpServer := &http.Server{
		Addr:    s.cfg.WSAddr,
		Handler: mux,
	}

	sub := s.hub.Subscribe()
	go s.broadcastLoop(ctx, sub)

	errCh := make(chan error, 1)
	go func() {
		errCh <- httpServer.ListenAndServe()
	}()

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		_ = httpServer.Shutdown(shutdownCtx)
		cancel()
		return nil
	case err := <-errCh:
		if err == http.ErrServerClosed {
			return nil
		}
		return err
	}
}

func (s *Server) handleWS(w http.ResponseWriter, r *http.Request) {
	upgrader := websocket.Upgrader{
		Subprotocols: []string{"foxglove.websocket.v1"},
		CheckOrigin: func(*http.Request) bool {
			return true
		},
	}
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}

	c := newClient(conn, s.cfg.SendBuf)
	s.addClient(c)

	if err := conn.WriteJSON(s.serverInfo()); err != nil {
		c.close()
		s.removeClient(c)
		return
	}
	if err := conn.WriteJSON(s.advertise()); err != nil {
		c.close()
		s.removeClient(c)
		return
	}

	go c.writeLoop()
	c.readLoop(s.cfg.ChannelID)

	c.close()
	s.removeClient(c)
}

func (s *Server) serverInfo() ServerInfoMsg {
	return ServerInfoMsg{
		Op:                 OpServerInfo,
		Name:               s.cfg.Name,
		Capabilities:       []string{},
		SupportedEncodings: []string{},
		SessionID:          fmt.Sprintf("%d", time.Now().UTC().UnixNano()),
	}
}

func (s *Server) advertise() AdvertiseMsg {
	return AdvertiseMsg{
		Op: OpAdvertise,
		Channels: []Channel{
			{
				ID:             s.cfg.ChannelID,
				Topic:          s.cfg.Topic,
				Encoding:       s.cfg.Encoding,
				SchemaName:     s.cfg.SchemaName,
				SchemaEncoding: s.cfg.SchemaEncoding,
				Schema:         s.cfg.Schema,
			},
		},
	}
}

func (s *Server) broadcastLoop(ctx context.Context, sub <-chan protocol.RatPacket) {
	for {
		select {
		case <-ctx.Done():
			return
		case pkt, ok := <-sub:
			if !ok {
				return
			}
			s.broadcastPacket(pkt)
		}
	}
}

func (s *Server) broadcastPacket(pkt protocol.RatPacket) {
	ts := pkt.Timestamp
	if ts.IsZero() {
		ts = time.Now()
	}

	rec := FoxglovePacket{
		ID:         formatID(pkt.ID),
		TS:         ts.UTC().Format(time.RFC3339Nano),
		PayloadHex: hex.EncodeToString(pkt.Payload),
		Data:       pkt.Data,
	}
	if pkt.ID == s.textID {
		if text, ok := pkt.Data.(string); ok {
			rec.Text = text
			rec.Data = nil
		}
	}

	payload, err := json.Marshal(rec)
	if err != nil {
		return
	}

	logTime := uint64(ts.UnixNano())
	clients := s.snapshotClients()
	for _, c := range clients {
		subIDs := c.subIDs()
		for _, subID := range subIDs {
			frame := EncodeMessageData(subID, logTime, payload)
			c.trySend(frame)
		}
	}
}

func (s *Server) addClient(c *client) {
	s.mu.Lock()
	s.clients[c] = struct{}{}
	s.mu.Unlock()
}

func (s *Server) removeClient(c *client) {
	s.mu.Lock()
	delete(s.clients, c)
	s.mu.Unlock()
}

func (s *Server) snapshotClients() []*client {
	s.mu.RLock()
	clients := make([]*client, 0, len(s.clients))
	for c := range s.clients {
		clients = append(clients, c)
	}
	s.mu.RUnlock()
	return clients
}

func newClient(conn *websocket.Conn, sendBuf int) *client {
	if sendBuf <= 0 {
		sendBuf = DefaultConfig().SendBuf
	}
	return &client{
		conn: conn,
		send: make(chan []byte, sendBuf),
		subs: make(map[uint32]struct{}),
	}
}

func (c *client) readLoop(channelID uint64) {
	for {
		msgType, data, err := c.conn.ReadMessage()
		if err != nil {
			return
		}
		if msgType != websocket.TextMessage {
			continue
		}

		var header struct {
			Op string `json:"op"`
		}
		if err := json.Unmarshal(data, &header); err != nil {
			continue
		}

		switch header.Op {
		case OpSubscribe:
			var msg SubscribeMsg
			if err := json.Unmarshal(data, &msg); err != nil {
				continue
			}
			for _, sub := range msg.Subscriptions {
				if sub.ChannelID == channelID {
					c.addSub(sub.ID)
				}
			}
		case OpUnsubscribe:
			var msg UnsubscribeMsg
			if err := json.Unmarshal(data, &msg); err != nil {
				continue
			}
			for _, id := range msg.SubscriptionIDs {
				c.removeSub(id)
			}
		}
	}
}

func (c *client) writeLoop() {
	for msg := range c.send {
		if err := c.conn.WriteMessage(websocket.BinaryMessage, msg); err != nil {
			c.close()
			return
		}
	}
}

func (c *client) trySend(msg []byte) {
	select {
	case c.send <- msg:
	default:
	}
}

func (c *client) addSub(id uint32) {
	c.mu.Lock()
	c.subs[id] = struct{}{}
	c.mu.Unlock()
}

func (c *client) removeSub(id uint32) {
	c.mu.Lock()
	delete(c.subs, id)
	c.mu.Unlock()
}

func (c *client) subIDs() []uint32 {
	c.mu.RLock()
	ids := make([]uint32, 0, len(c.subs))
	for id := range c.subs {
		ids = append(ids, id)
	}
	c.mu.RUnlock()
	return ids
}

func (c *client) close() {
	c.once.Do(func() {
		close(c.send)
		_ = c.conn.Close()
	})
}

func formatID(id uint8) string {
	return fmt.Sprintf("0x%02x", id)
}

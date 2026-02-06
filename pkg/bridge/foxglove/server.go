package foxglove

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"net/http"
	"os"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

const (
	markerTypeCube       = 1
	markerActionAdd      = 0
	logLevelInfo         = 2
	imagePublishInterval = time.Second
)

type FoxglovePacket struct {
	ID         string `json:"id"`
	TS         string `json:"ts,omitempty"`
	PayloadHex string `json:"payload_hex"`
	Data       any    `json:"data,omitempty"`
	Text       string `json:"text,omitempty"`
}

type MarkerMessage struct {
	Header MarkerHeader `json:"header"`
	NS     string       `json:"ns"`
	ID     int32        `json:"id"`
	Type   int32        `json:"type"`
	Action int32        `json:"action"`
	Pose   MarkerPose   `json:"pose"`
	Scale  Vector3      `json:"scale"`
	Color  ColorRGBA    `json:"color"`
}

type MarkerHeader struct {
	FrameID string      `json:"frame_id"`
	Stamp   MarkerStamp `json:"stamp"`
}

type MarkerStamp struct {
	Sec  int64 `json:"sec"`
	Nsec int64 `json:"nsec"`
}

type MarkerPose struct {
	Position    Vector3     `json:"position"`
	Orientation Quaternion3 `json:"orientation"`
}

type Vector3 struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
	Z float64 `json:"z"`
}

type Quaternion3 struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
	Z float64 `json:"z"`
	W float64 `json:"w"`
}

type ColorRGBA struct {
	R float64 `json:"r"`
	G float64 `json:"g"`
	B float64 `json:"b"`
	A float64 `json:"a"`
}

type FrameTransformMessage struct {
	Timestamp     FrameTime   `json:"timestamp"`
	ParentFrameID string      `json:"parent_frame_id"`
	ChildFrameID  string      `json:"child_frame_id"`
	Translation   Vector3     `json:"translation"`
	Rotation      Quaternion3 `json:"rotation"`
}

type FrameTransformsMessage struct {
	Transforms []FrameTransformMessage `json:"transforms"`
}

type FrameTime struct {
	Sec  uint32 `json:"sec"`
	Nsec uint32 `json:"nsec"`
}

type CompressedImageMessage struct {
	Timestamp FrameTime `json:"timestamp"`
	FrameID   string    `json:"frame_id"`
	Format    string    `json:"format"`
	Data      string    `json:"data"`
}

type LogMessage struct {
	Timestamp FrameTime `json:"timestamp"`
	Level     uint8     `json:"level"`
	Message   string    `json:"message"`
	Name      string    `json:"name"`
	File      string    `json:"file"`
	Line      uint32    `json:"line"`
}

type TemperatureMessage struct {
	Timestamp FrameTime `json:"timestamp"`
	Value     float64   `json:"value"`
	Unit      string    `json:"unit"`
}

type Server struct {
	cfg          Config
	hub          *engine.Hub
	textID       uint8
	quatID       uint8
	clients      map[*client]struct{}
	imageEnabled bool
	imagePayload string
	mu           sync.RWMutex
}

type client struct {
	conn *websocket.Conn
	send chan []byte
	subs map[uint32]uint64
	mu   sync.RWMutex
	once sync.Once
}

func NewServer(cfg Config, hub *engine.Hub, textID uint8, quatID uint8) *Server {
	defaults := DefaultConfig()
	if cfg.WSAddr == "" {
		cfg.WSAddr = defaults.WSAddr
	}
	if cfg.Name == "" {
		cfg.Name = defaults.Name
	}
	if cfg.Topic == "" {
		cfg.Topic = defaults.Topic
	}
	if cfg.ChannelID == 0 {
		cfg.ChannelID = defaults.ChannelID
	}
	if cfg.SchemaName == "" {
		cfg.SchemaName = defaults.SchemaName
	}
	if cfg.SchemaEncoding == "" {
		cfg.SchemaEncoding = defaults.SchemaEncoding
	}
	if cfg.Schema == "" {
		cfg.Schema = defaults.Schema
	}
	if cfg.Encoding == "" {
		cfg.Encoding = defaults.Encoding
	}
	if cfg.MarkerTopic == "" {
		cfg.MarkerTopic = defaults.MarkerTopic
	}
	if cfg.MarkerChannelID == 0 {
		cfg.MarkerChannelID = defaults.MarkerChannelID
	}
	if cfg.MarkerSchemaName == "" {
		cfg.MarkerSchemaName = defaults.MarkerSchemaName
	}
	if cfg.MarkerSchemaEncoding == "" {
		cfg.MarkerSchemaEncoding = defaults.MarkerSchemaEncoding
	}
	if cfg.MarkerSchema == "" {
		cfg.MarkerSchema = defaults.MarkerSchema
	}
	if cfg.MarkerEncoding == "" {
		cfg.MarkerEncoding = defaults.MarkerEncoding
	}
	if cfg.TransformTopic == "" {
		cfg.TransformTopic = defaults.TransformTopic
	}
	if cfg.TransformChannelID == 0 {
		cfg.TransformChannelID = defaults.TransformChannelID
	}
	if cfg.TransformSchemaName == "" {
		cfg.TransformSchemaName = defaults.TransformSchemaName
	}
	if cfg.TransformSchemaEncoding == "" {
		cfg.TransformSchemaEncoding = defaults.TransformSchemaEncoding
	}
	if cfg.TransformSchema == "" {
		cfg.TransformSchema = defaults.TransformSchema
	}
	if cfg.TransformEncoding == "" {
		cfg.TransformEncoding = defaults.TransformEncoding
	}
	if cfg.ImageTopic == "" {
		cfg.ImageTopic = defaults.ImageTopic
	}
	if cfg.ImageChannelID == 0 {
		cfg.ImageChannelID = defaults.ImageChannelID
	}
	if cfg.ImageSchemaName == "" {
		cfg.ImageSchemaName = defaults.ImageSchemaName
	}
	if cfg.ImageSchemaEncoding == "" {
		cfg.ImageSchemaEncoding = defaults.ImageSchemaEncoding
	}
	if cfg.ImageSchema == "" {
		cfg.ImageSchema = defaults.ImageSchema
	}
	if cfg.ImageEncoding == "" {
		cfg.ImageEncoding = defaults.ImageEncoding
	}
	if cfg.ImageFrameID == "" {
		cfg.ImageFrameID = defaults.ImageFrameID
	}
	if cfg.ImageFormat == "" {
		cfg.ImageFormat = defaults.ImageFormat
	}
	if cfg.LogTopic == "" {
		cfg.LogTopic = defaults.LogTopic
	}
	if cfg.LogChannelID == 0 {
		cfg.LogChannelID = defaults.LogChannelID
	}
	if cfg.LogSchemaName == "" {
		cfg.LogSchemaName = defaults.LogSchemaName
	}
	if cfg.LogSchemaEncoding == "" {
		cfg.LogSchemaEncoding = defaults.LogSchemaEncoding
	}
	if cfg.LogSchema == "" {
		cfg.LogSchema = defaults.LogSchema
	}
	if cfg.LogEncoding == "" {
		cfg.LogEncoding = defaults.LogEncoding
	}
	if cfg.LogName == "" {
		cfg.LogName = defaults.LogName
	}
	if cfg.TempTopic == "" {
		cfg.TempTopic = defaults.TempTopic
	}
	if cfg.TempChannelID == 0 {
		cfg.TempChannelID = defaults.TempChannelID
	}
	if cfg.TempSchemaName == "" {
		cfg.TempSchemaName = defaults.TempSchemaName
	}
	if cfg.TempSchemaEncoding == "" {
		cfg.TempSchemaEncoding = defaults.TempSchemaEncoding
	}
	if cfg.TempSchema == "" {
		cfg.TempSchema = defaults.TempSchema
	}
	if cfg.TempEncoding == "" {
		cfg.TempEncoding = defaults.TempEncoding
	}
	if cfg.TempUnit == "" {
		cfg.TempUnit = defaults.TempUnit
	}
	if cfg.ParentFrameID == "" {
		cfg.ParentFrameID = defaults.ParentFrameID
	}
	if cfg.FrameID == "" {
		cfg.FrameID = defaults.FrameID
	}
	if cfg.MarkerChannelID == cfg.ChannelID {
		cfg.MarkerChannelID = cfg.ChannelID + 1
	}
	if cfg.TransformChannelID == cfg.ChannelID || cfg.TransformChannelID == cfg.MarkerChannelID {
		cfg.TransformChannelID = maxUint64(cfg.ChannelID, cfg.MarkerChannelID) + 1
	}
	if cfg.ImageChannelID == cfg.ChannelID || cfg.ImageChannelID == cfg.MarkerChannelID || cfg.ImageChannelID == cfg.TransformChannelID {
		cfg.ImageChannelID = maxUint64(cfg.TransformChannelID, maxUint64(cfg.ChannelID, cfg.MarkerChannelID)) + 1
	}
	if cfg.LogChannelID == cfg.ChannelID || cfg.LogChannelID == cfg.MarkerChannelID || cfg.LogChannelID == cfg.TransformChannelID || cfg.LogChannelID == cfg.ImageChannelID {
		cfg.LogChannelID = maxUint64(cfg.ImageChannelID, maxUint64(cfg.TransformChannelID, maxUint64(cfg.ChannelID, cfg.MarkerChannelID))) + 1
	}
	if cfg.TempChannelID == cfg.ChannelID || cfg.TempChannelID == cfg.MarkerChannelID || cfg.TempChannelID == cfg.TransformChannelID || cfg.TempChannelID == cfg.ImageChannelID || cfg.TempChannelID == cfg.LogChannelID {
		cfg.TempChannelID = maxUint64(cfg.LogChannelID, maxUint64(cfg.ImageChannelID, maxUint64(cfg.TransformChannelID, maxUint64(cfg.ChannelID, cfg.MarkerChannelID)))) + 1
	}
	if cfg.SendBuf <= 0 {
		cfg.SendBuf = defaults.SendBuf
	}

	imageEnabled := false
	imagePayload := ""
	if cfg.ImagePath != "" {
		if content, err := os.ReadFile(cfg.ImagePath); err == nil && len(content) > 0 {
			imagePayload = base64.StdEncoding.EncodeToString(content)
			imageEnabled = true
		}
	}

	return &Server{
		cfg:          cfg,
		hub:          hub,
		textID:       textID,
		quatID:       quatID,
		clients:      make(map[*client]struct{}),
		imageEnabled: imageEnabled,
		imagePayload: imagePayload,
	}
}

func (s *Server) Run(ctx context.Context) error {
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleWS)

	httpServer := &http.Server{
		Addr:    s.cfg.WSAddr,
		Handler: mux,
	}

	sub := s.hub.Subscribe()
	go s.broadcastLoop(ctx, sub)
	if s.imageEnabled {
		go s.imageLoop(ctx)
	}

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
	c.readLoop(s.supportedChannels())

	c.close()
	s.removeClient(c)
}

func (s *Server) supportedChannels() map[uint64]struct{} {
	channels := map[uint64]struct{}{
		s.cfg.ChannelID:          {},
		s.cfg.MarkerChannelID:    {},
		s.cfg.TransformChannelID: {},
		s.cfg.LogChannelID:       {},
		s.cfg.TempChannelID:      {},
	}
	if s.imageEnabled {
		channels[s.cfg.ImageChannelID] = struct{}{}
	}
	return channels
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
	channels := []Channel{
		{
			ID:             s.cfg.ChannelID,
			Topic:          s.cfg.Topic,
			Encoding:       s.cfg.Encoding,
			SchemaName:     s.cfg.SchemaName,
			SchemaEncoding: s.cfg.SchemaEncoding,
			Schema:         s.cfg.Schema,
		},
		{
			ID:             s.cfg.MarkerChannelID,
			Topic:          s.cfg.MarkerTopic,
			Encoding:       s.cfg.MarkerEncoding,
			SchemaName:     s.cfg.MarkerSchemaName,
			SchemaEncoding: s.cfg.MarkerSchemaEncoding,
			Schema:         s.cfg.MarkerSchema,
		},
		{
			ID:             s.cfg.TransformChannelID,
			Topic:          s.cfg.TransformTopic,
			Encoding:       s.cfg.TransformEncoding,
			SchemaName:     s.cfg.TransformSchemaName,
			SchemaEncoding: s.cfg.TransformSchemaEncoding,
			Schema:         s.cfg.TransformSchema,
		},
		{
			ID:             s.cfg.LogChannelID,
			Topic:          s.cfg.LogTopic,
			Encoding:       s.cfg.LogEncoding,
			SchemaName:     s.cfg.LogSchemaName,
			SchemaEncoding: s.cfg.LogSchemaEncoding,
			Schema:         s.cfg.LogSchema,
		},
		{
			ID:             s.cfg.TempChannelID,
			Topic:          s.cfg.TempTopic,
			Encoding:       s.cfg.TempEncoding,
			SchemaName:     s.cfg.TempSchemaName,
			SchemaEncoding: s.cfg.TempSchemaEncoding,
			Schema:         s.cfg.TempSchema,
		},
	}
	if s.imageEnabled {
		channels = append(channels, Channel{
			ID:             s.cfg.ImageChannelID,
			Topic:          s.cfg.ImageTopic,
			Encoding:       s.cfg.ImageEncoding,
			SchemaName:     s.cfg.ImageSchemaName,
			SchemaEncoding: s.cfg.ImageSchemaEncoding,
			Schema:         s.cfg.ImageSchema,
		})
	}
	return AdvertiseMsg{Op: OpAdvertise, Channels: channels}
}

func (s *Server) imageLoop(ctx context.Context) {
	s.publishImage(time.Now())
	ticker := time.NewTicker(imagePublishInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case ts := <-ticker.C:
			s.publishImage(ts)
		}
	}
}

func (s *Server) publishImage(ts time.Time) {
	msg, ok := s.compressedImage(ts)
	if !ok {
		return
	}
	s.publishJSONToChannel(s.cfg.ImageChannelID, ts, msg)
}

func (s *Server) compressedImage(ts time.Time) (CompressedImageMessage, bool) {
	if !s.imageEnabled || s.imagePayload == "" {
		return CompressedImageMessage{}, false
	}
	return CompressedImageMessage{
		Timestamp: FrameTime{Sec: uint32(ts.Unix()), Nsec: uint32(ts.Nanosecond())},
		FrameID:   s.cfg.ImageFrameID,
		Format:    s.cfg.ImageFormat,
		Data:      s.imagePayload,
	}, true
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
		text, ok := pkt.Data.(string)
		if !ok {
			text = protocol.ParseText(pkt.Payload)
		}
		rec.Text = text
		rec.Data = nil
	}
	s.publishJSONToChannel(s.cfg.ChannelID, ts, rec)

	if log, ok := s.logFromPacket(pkt, ts); ok {
		s.publishJSONToChannel(s.cfg.LogChannelID, ts, log)
	}
	if temp, ok := s.temperatureFromPacket(pkt, ts); ok {
		s.publishJSONToChannel(s.cfg.TempChannelID, ts, temp)
	}
	if marker, ok := s.markerFromPacket(pkt, ts); ok {
		s.publishJSONToChannel(s.cfg.MarkerChannelID, ts, marker)
	}
	if transform, ok := s.transformFromPacket(pkt, ts); ok {
		s.publishJSONToChannel(s.cfg.TransformChannelID, ts, transform)
	}
}

func (s *Server) publishJSONToChannel(channelID uint64, ts time.Time, message any) {
	payload, err := json.Marshal(message)
	if err != nil {
		return
	}

	logTime := uint64(ts.UnixNano())
	clients := s.snapshotClients()
	for _, c := range clients {
		subIDs := c.subIDsForChannel(channelID)
		for _, subID := range subIDs {
			frame := EncodeMessageData(subID, logTime, payload)
			c.trySend(frame)
		}
	}
}

func (s *Server) logFromPacket(pkt protocol.RatPacket, ts time.Time) (LogMessage, bool) {
	if pkt.ID != s.textID {
		return LogMessage{}, false
	}

	text, ok := pkt.Data.(string)
	if !ok {
		text = protocol.ParseText(pkt.Payload)
	}

	return LogMessage{
		Timestamp: FrameTime{Sec: uint32(ts.Unix()), Nsec: uint32(ts.Nanosecond())},
		Level:     logLevelInfo,
		Message:   text,
		Name:      s.cfg.LogName,
		File:      "",
		Line:      0,
	}, true
}

func (s *Server) temperatureFromPacket(pkt protocol.RatPacket, ts time.Time) (TemperatureMessage, bool) {
	temp, ok := pkt.Data.(protocol.TemperaturePacket)
	if !ok {
		return TemperatureMessage{}, false
	}
	return TemperatureMessage{
		Timestamp: FrameTime{Sec: uint32(ts.Unix()), Nsec: uint32(ts.Nanosecond())},
		Value:     float64(temp.Celsius),
		Unit:      s.cfg.TempUnit,
	}, true
}

func (s *Server) markerFromPacket(pkt protocol.RatPacket, ts time.Time) (MarkerMessage, bool) {
	if pkt.ID != s.quatID {
		return MarkerMessage{}, false
	}

	quat, ok := extractQuaternion(pkt)
	if !ok {
		return MarkerMessage{}, false
	}

	return MarkerMessage{
		Header: MarkerHeader{
			FrameID: s.cfg.FrameID,
			Stamp: MarkerStamp{
				Sec:  ts.Unix(),
				Nsec: int64(ts.Nanosecond()),
			},
		},
		NS:     "ratitude.imu",
		ID:     1,
		Type:   markerTypeCube,
		Action: markerActionAdd,
		Pose: MarkerPose{
			Position: Vector3{X: 0, Y: 0, Z: 0},
			Orientation: Quaternion3{
				X: float64(quat.X),
				Y: float64(quat.Y),
				Z: float64(quat.Z),
				W: float64(quat.W),
			},
		},
		Scale: Vector3{X: 0.3, Y: 0.3, Z: 0.3},
		Color: ColorRGBA{R: 1, G: 1, B: 1, A: 1},
	}, true
}

func (s *Server) transformFromPacket(pkt protocol.RatPacket, ts time.Time) (FrameTransformsMessage, bool) {
	if pkt.ID != s.quatID {
		return FrameTransformsMessage{}, false
	}

	quat, ok := extractQuaternion(pkt)
	if !ok {
		return FrameTransformsMessage{}, false
	}

	transform := FrameTransformMessage{
		Timestamp: FrameTime{
			Sec:  uint32(ts.Unix()),
			Nsec: uint32(ts.Nanosecond()),
		},
		ParentFrameID: s.cfg.ParentFrameID,
		ChildFrameID:  s.cfg.FrameID,
		Translation:   Vector3{X: 0, Y: 0, Z: 0},
		Rotation: Quaternion3{
			X: float64(quat.X),
			Y: float64(quat.Y),
			Z: float64(quat.Z),
			W: float64(quat.W),
		},
	}
	return FrameTransformsMessage{Transforms: []FrameTransformMessage{transform}}, true
}

func extractQuaternion(pkt protocol.RatPacket) (protocol.QuatPacket, bool) {
	if quat, ok := pkt.Data.(protocol.QuatPacket); ok {
		return quat, true
	}
	if m, ok := pkt.Data.(map[string]any); ok {
		if quat, ok := quaternionFromMap(m); ok {
			return quat, true
		}
	}
	if len(pkt.Payload) < 16 {
		return protocol.QuatPacket{}, false
	}
	return protocol.QuatPacket{
		W: math.Float32frombits(binary.LittleEndian.Uint32(pkt.Payload[0:4])),
		X: math.Float32frombits(binary.LittleEndian.Uint32(pkt.Payload[4:8])),
		Y: math.Float32frombits(binary.LittleEndian.Uint32(pkt.Payload[8:12])),
		Z: math.Float32frombits(binary.LittleEndian.Uint32(pkt.Payload[12:16])),
	}, true
}

func quaternionFromMap(data map[string]any) (protocol.QuatPacket, bool) {
	x, okX := numberToFloat32(data["x"])
	y, okY := numberToFloat32(data["y"])
	z, okZ := numberToFloat32(data["z"])
	w, okW := numberToFloat32(data["w"])
	if okX && okY && okZ && okW {
		return protocol.QuatPacket{X: x, Y: y, Z: z, W: w}, true
	}

	w2, okW2 := numberToFloat32(data["q_w"])
	x2, okX2 := numberToFloat32(data["q_x"])
	y2, okY2 := numberToFloat32(data["q_y"])
	z2, okZ2 := numberToFloat32(data["q_z"])
	if okW2 && okX2 && okY2 && okZ2 {
		return protocol.QuatPacket{W: w2, X: x2, Y: y2, Z: z2}, true
	}

	return protocol.QuatPacket{}, false
}

func numberToFloat32(v any) (float32, bool) {
	switch n := v.(type) {
	case float32:
		return n, true
	case float64:
		return float32(n), true
	case int:
		return float32(n), true
	case int8:
		return float32(n), true
	case int16:
		return float32(n), true
	case int32:
		return float32(n), true
	case int64:
		return float32(n), true
	case uint:
		return float32(n), true
	case uint8:
		return float32(n), true
	case uint16:
		return float32(n), true
	case uint32:
		return float32(n), true
	case uint64:
		return float32(n), true
	default:
		return 0, false
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
		subs: make(map[uint32]uint64),
	}
}

func (c *client) readLoop(supportedChannels map[uint64]struct{}) {
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
				if _, ok := supportedChannels[sub.ChannelID]; ok {
					c.addSub(sub.ID, sub.ChannelID)
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
	defer func() {
		_ = recover()
	}()
	select {
	case c.send <- msg:
	default:
	}
}

func (c *client) addSub(id uint32, channelID uint64) {
	c.mu.Lock()
	c.subs[id] = channelID
	c.mu.Unlock()
}

func (c *client) removeSub(id uint32) {
	c.mu.Lock()
	delete(c.subs, id)
	c.mu.Unlock()
}

func (c *client) subIDsForChannel(channelID uint64) []uint32 {
	c.mu.RLock()
	ids := make([]uint32, 0, len(c.subs))
	for id, ch := range c.subs {
		if ch == channelID {
			ids = append(ids, id)
		}
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

func maxUint64(a uint64, b uint64) uint64 {
	if a > b {
		return a
	}
	return b
}

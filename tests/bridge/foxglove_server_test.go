package bridge_test

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"net"
	"net/url"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"ratitude/pkg/bridge/foxglove"
	"ratitude/pkg/engine"
	"ratitude/pkg/protocol"
)

type foxgloveSession struct {
	hub      *engine.Hub
	conn     *websocket.Conn
	cancel   context.CancelFunc
	channels map[string]foxglove.Channel
}

func startFoxgloveSession(t *testing.T, cfg foxglove.Config, textID uint8, quatID uint8) *foxgloveSession {
	t.Helper()

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen free port: %v", err)
	}
	cfg.WSAddr = ln.Addr().String()
	_ = ln.Close()

	ctx, cancel := context.WithCancel(context.Background())
	hub := engine.NewHub()
	go hub.Run(ctx)

	srv := foxglove.NewServer(cfg, hub, textID, quatID)
	errCh := make(chan error, 1)
	go func() {
		errCh <- srv.Run(ctx)
	}()

	dialURL := url.URL{Scheme: "ws", Host: cfg.WSAddr, Path: "/"}
	dialer := websocket.Dialer{Subprotocols: []string{"foxglove.websocket.v1"}}

	var conn *websocket.Conn
	for i := 0; i < 80; i++ {
		conn, _, err = dialer.Dial(dialURL.String(), nil)
		if err == nil {
			break
		}
		time.Sleep(25 * time.Millisecond)
	}
	if err != nil {
		cancel()
		t.Fatalf("dial foxglove websocket: %v", err)
	}

	if _, infoRaw, err := readWSMessage(conn); err != nil {
		cancel()
		_ = conn.Close()
		t.Fatalf("read serverInfo: %v", err)
	} else {
		var info map[string]any
		if err := json.Unmarshal(infoRaw, &info); err != nil {
			cancel()
			_ = conn.Close()
			t.Fatalf("decode serverInfo json: %v", err)
		}
		if op, _ := info["op"].(string); op != foxglove.OpServerInfo {
			cancel()
			_ = conn.Close()
			t.Fatalf("unexpected first op: %v", info["op"])
		}
	}

	_, advRaw, err := readWSMessage(conn)
	if err != nil {
		cancel()
		_ = conn.Close()
		t.Fatalf("read advertise: %v", err)
	}
	var adv foxglove.AdvertiseMsg
	if err := json.Unmarshal(advRaw, &adv); err != nil {
		cancel()
		_ = conn.Close()
		t.Fatalf("decode advertise json: %v", err)
	}

	channels := make(map[string]foxglove.Channel, len(adv.Channels))
	for _, ch := range adv.Channels {
		channels[ch.Topic] = ch
	}

	t.Cleanup(func() {
		_ = conn.Close()
		cancel()
		select {
		case err := <-errCh:
			if err != nil {
				t.Fatalf("foxglove server run error: %v", err)
			}
		case <-time.After(3 * time.Second):
			t.Fatalf("timed out waiting foxglove server shutdown")
		}
	})

	return &foxgloveSession{
		hub:      hub,
		conn:     conn,
		cancel:   cancel,
		channels: channels,
	}
}

func readWSMessage(conn *websocket.Conn) (int, []byte, error) {
	_ = conn.SetReadDeadline(time.Now().Add(2 * time.Second))
	msgType, raw, err := conn.ReadMessage()
	_ = conn.SetReadDeadline(time.Time{})
	return msgType, raw, err
}

func subscribeChannel(t *testing.T, conn *websocket.Conn, subID uint32, channelID uint64) {
	t.Helper()
	msg := foxglove.SubscribeMsg{
		Op: foxglove.OpSubscribe,
		Subscriptions: []foxglove.Subscription{
			{ID: subID, ChannelID: channelID},
		},
	}
	if err := conn.WriteJSON(msg); err != nil {
		t.Fatalf("subscribe channel %d: %v", channelID, err)
	}
	// Server subscriptions are async; give it a short window before publishing.
	time.Sleep(20 * time.Millisecond)
}

func readBinaryPayloadForSubID(t *testing.T, conn *websocket.Conn, subID uint32) []byte {
	t.Helper()
	for i := 0; i < 40; i++ {
		msgType, frame, err := readWSMessage(conn)
		if err != nil {
			t.Fatalf("read messageData frame: %v", err)
		}
		if msgType != websocket.BinaryMessage {
			continue
		}
		if len(frame) < 13 || frame[0] != foxglove.BinaryOpMessageData {
			continue
		}
		gotSubID := binary.LittleEndian.Uint32(frame[1:5])
		if gotSubID != subID {
			continue
		}
		payload := make([]byte, len(frame[13:]))
		copy(payload, frame[13:])
		return payload
	}
	t.Fatalf("did not receive messageData for subscription id %d", subID)
	return nil
}

func TestFoxgloveAdvertiseChannelsWithoutImage(t *testing.T) {
	cfg := foxglove.DefaultConfig()
	cfg.ImagePath = ""

	s := startFoxgloveSession(t, cfg, 0xFF, 0x10)

	for _, topic := range []string{cfg.Topic, cfg.MarkerTopic, cfg.TransformTopic, cfg.LogTopic, cfg.TempTopic} {
		if _, ok := s.channels[topic]; !ok {
			t.Fatalf("missing advertised topic: %s", topic)
		}
	}
	if len(s.channels) != 5 {
		t.Fatalf("expected 5 channels without image, got %d", len(s.channels))
	}
	if ch := s.channels[cfg.LogTopic]; ch.SchemaName != "foxglove.Log" {
		t.Fatalf("unexpected log schema: %s", ch.SchemaName)
	}
}

func TestFoxglovePublishesLogPanelMessage(t *testing.T) {
	cfg := foxglove.DefaultConfig()
	cfg.ImagePath = ""

	s := startFoxgloveSession(t, cfg, 0xFF, 0x10)
	logChannel := s.channels[cfg.LogTopic]
	subscribeChannel(t, s.conn, 11, logChannel.ID)

	s.hub.Publish(protocol.RatPacket{
		ID:        0xFF,
		Timestamp: time.Unix(123, 456),
		Payload:   []byte("rat_info hello"),
		Data:      "rat_info hello",
	})

	payload := readBinaryPayloadForSubID(t, s.conn, 11)
	var rec struct {
		Level   uint8  `json:"level"`
		Message string `json:"message"`
		Name    string `json:"name"`
	}
	if err := json.Unmarshal(payload, &rec); err != nil {
		t.Fatalf("decode log payload: %v", err)
	}
	if rec.Level != 2 {
		t.Fatalf("unexpected log level: %d", rec.Level)
	}
	if rec.Message != "rat_info hello" {
		t.Fatalf("unexpected log message: %s", rec.Message)
	}
	if rec.Name != cfg.LogName {
		t.Fatalf("unexpected log name: %s", rec.Name)
	}
}

func TestFoxglovePublishesTemperatureGaugeMessage(t *testing.T) {
	cfg := foxglove.DefaultConfig()
	cfg.ImagePath = ""

	s := startFoxgloveSession(t, cfg, 0xFF, 0x10)
	tempChannel := s.channels[cfg.TempTopic]
	subscribeChannel(t, s.conn, 22, tempChannel.ID)

	s.hub.Publish(protocol.RatPacket{
		ID:        0x20,
		Timestamp: time.Unix(321, 654),
		Payload:   []byte{0, 0, 0, 0},
		Data:      protocol.TemperaturePacket{Celsius: 38.5},
	})

	payload := readBinaryPayloadForSubID(t, s.conn, 22)
	var rec struct {
		Value float64 `json:"value"`
		Unit  string  `json:"unit"`
	}
	if err := json.Unmarshal(payload, &rec); err != nil {
		t.Fatalf("decode temperature payload: %v", err)
	}
	if rec.Value != 38.5 {
		t.Fatalf("unexpected temperature value: %.3f", rec.Value)
	}
	if rec.Unit != cfg.TempUnit {
		t.Fatalf("unexpected temperature unit: %s", rec.Unit)
	}
}

func TestFoxglovePublishesPose3DFromDynamicMap(t *testing.T) {
	cfg := foxglove.DefaultConfig()
	cfg.ImagePath = ""

	s := startFoxgloveSession(t, cfg, 0xFF, 0x10)
	markerChannel := s.channels[cfg.MarkerTopic]
	transformChannel := s.channels[cfg.TransformTopic]
	subscribeChannel(t, s.conn, 31, markerChannel.ID)
	subscribeChannel(t, s.conn, 32, transformChannel.ID)

	s.hub.Publish(protocol.RatPacket{
		ID:        0x10,
		Timestamp: time.Unix(777, 999),
		Data: map[string]any{
			"x": 0.1,
			"y": -0.2,
			"z": 0.3,
			"w": 0.9,
		},
	})

	markerPayload := readBinaryPayloadForSubID(t, s.conn, 31)
	var marker struct {
		Pose struct {
			Orientation struct {
				X float64 `json:"x"`
				Y float64 `json:"y"`
				Z float64 `json:"z"`
				W float64 `json:"w"`
			} `json:"orientation"`
		} `json:"pose"`
	}
	if err := json.Unmarshal(markerPayload, &marker); err != nil {
		t.Fatalf("decode marker payload: %v", err)
	}
	if !closeEnough(marker.Pose.Orientation.X, 0.1) || !closeEnough(marker.Pose.Orientation.Y, -0.2) || !closeEnough(marker.Pose.Orientation.Z, 0.3) || !closeEnough(marker.Pose.Orientation.W, 0.9) {
		t.Fatalf("unexpected marker quaternion: %+v", marker.Pose.Orientation)
	}

	transformPayload := readBinaryPayloadForSubID(t, s.conn, 32)
	var tf struct {
		Transforms []struct {
			Rotation struct {
				X float64 `json:"x"`
				Y float64 `json:"y"`
				Z float64 `json:"z"`
				W float64 `json:"w"`
			} `json:"rotation"`
		} `json:"transforms"`
	}
	if err := json.Unmarshal(transformPayload, &tf); err != nil {
		t.Fatalf("decode transform payload: %v", err)
	}
	if len(tf.Transforms) != 1 {
		t.Fatalf("expected one transform, got %d", len(tf.Transforms))
	}
	if !closeEnough(tf.Transforms[0].Rotation.X, 0.1) || !closeEnough(tf.Transforms[0].Rotation.Y, -0.2) || !closeEnough(tf.Transforms[0].Rotation.Z, 0.3) || !closeEnough(tf.Transforms[0].Rotation.W, 0.9) {
		t.Fatalf("unexpected transform quaternion: %+v", tf.Transforms[0].Rotation)
	}
}

func closeEnough(got float64, want float64) bool {
	delta := got - want
	if delta < 0 {
		delta = -delta
	}
	return delta < 1e-6
}

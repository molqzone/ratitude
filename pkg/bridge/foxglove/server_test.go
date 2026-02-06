package foxglove

import (
	"encoding/base64"
	"encoding/binary"
	"math"
	"os"
	"path/filepath"
	"testing"
	"time"

	"ratitude/pkg/protocol"
)

func TestExtractQuaternionFromStruct(t *testing.T) {
	pkt := protocol.RatPacket{Data: protocol.QuatPacket{W: 1, X: 2, Y: 3, Z: 4}}
	quat, ok := extractQuaternion(pkt)
	if !ok {
		t.Fatalf("expected quaternion to decode")
	}
	if quat.W != 1 || quat.X != 2 || quat.Y != 3 || quat.Z != 4 {
		t.Fatalf("unexpected quaternion: %+v", quat)
	}
}

func TestExtractQuaternionFromPayload(t *testing.T) {
	payload := make([]byte, 16)
	binary.LittleEndian.PutUint32(payload[0:4], math.Float32bits(0.5))
	binary.LittleEndian.PutUint32(payload[4:8], math.Float32bits(0.25))
	binary.LittleEndian.PutUint32(payload[8:12], math.Float32bits(-0.25))
	binary.LittleEndian.PutUint32(payload[12:16], math.Float32bits(-0.5))
	pkt := protocol.RatPacket{Payload: payload}

	quat, ok := extractQuaternion(pkt)
	if !ok {
		t.Fatalf("expected quaternion payload to decode")
	}
	if quat.W != 0.5 || quat.X != 0.25 || quat.Y != -0.25 || quat.Z != -0.5 {
		t.Fatalf("unexpected quaternion: %+v", quat)
	}
}

func TestMarkerFromPacket(t *testing.T) {
	srv := NewServer(DefaultConfig(), nil, 0xFF, 0x10)
	ts := time.Unix(10, 123)
	pkt := protocol.RatPacket{
		ID:        0x10,
		Timestamp: ts,
		Data:      protocol.QuatPacket{W: 1, X: 0, Y: 0, Z: 0},
	}

	marker, ok := srv.markerFromPacket(pkt, ts)
	if !ok {
		t.Fatalf("expected marker to be created")
	}
	if marker.Header.FrameID != srv.cfg.FrameID {
		t.Fatalf("unexpected frame id: %s", marker.Header.FrameID)
	}
	if marker.Type != markerTypeCube || marker.Action != markerActionAdd {
		t.Fatalf("unexpected marker mode: type=%d action=%d", marker.Type, marker.Action)
	}
	if marker.Pose.Orientation.W != 1 {
		t.Fatalf("unexpected marker orientation: %+v", marker.Pose.Orientation)
	}
	if marker.Scale.X != marker.Scale.Y || marker.Scale.Y != marker.Scale.Z {
		t.Fatalf("expected cube scale, got %+v", marker.Scale)
	}
	if marker.Color.R != 1 || marker.Color.G != 1 || marker.Color.B != 1 || marker.Color.A != 1 {
		t.Fatalf("expected white cube, got %+v", marker.Color)
	}
}

func TestTransformFromPacket(t *testing.T) {
	srv := NewServer(DefaultConfig(), nil, 0xFF, 0x10)
	ts := time.Unix(42, 99)
	pkt := protocol.RatPacket{
		ID:        0x10,
		Timestamp: ts,
		Data:      protocol.QuatPacket{W: 1, X: 0, Y: 0, Z: 0},
	}

	tf, ok := srv.transformFromPacket(pkt, ts)
	if !ok {
		t.Fatalf("expected transform message")
	}
	if len(tf.Transforms) != 1 {
		t.Fatalf("expected one transform, got %d", len(tf.Transforms))
	}
	tr := tf.Transforms[0]
	if tr.ParentFrameID != srv.cfg.ParentFrameID || tr.ChildFrameID != srv.cfg.FrameID {
		t.Fatalf("unexpected frame chain: %+v", tr)
	}
	if tr.Rotation.W != 1 || tr.Rotation.X != 0 || tr.Rotation.Y != 0 || tr.Rotation.Z != 0 {
		t.Fatalf("unexpected transform rotation: %+v", tr.Rotation)
	}
}

func TestAdvertiseIncludesMarkerAndTransformChannels(t *testing.T) {
	cfg := DefaultConfig()
	cfg.ImagePath = ""
	srv := NewServer(cfg, nil, 0xFF, 0x10)
	msg := srv.advertise()
	if len(msg.Channels) != 3 {
		t.Fatalf("expected 3 channels, got %d", len(msg.Channels))
	}
	if msg.Channels[0].ID != srv.cfg.ChannelID {
		t.Fatalf("unexpected packet channel id: %d", msg.Channels[0].ID)
	}
	if msg.Channels[1].ID != srv.cfg.MarkerChannelID {
		t.Fatalf("unexpected marker channel id: %d", msg.Channels[1].ID)
	}
	if msg.Channels[2].ID != srv.cfg.TransformChannelID {
		t.Fatalf("unexpected transform channel id: %d", msg.Channels[2].ID)
	}
	if msg.Channels[2].Topic != srv.cfg.TransformTopic {
		t.Fatalf("unexpected transform topic: %s", msg.Channels[2].Topic)
	}
}

func TestAdvertiseIncludesImageChannelWhenEnabled(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "demo.jpg")
	if err := os.WriteFile(imagePath, []byte{0xFF, 0xD8, 0xFF, 0xD9}, 0o600); err != nil {
		t.Fatalf("write image: %v", err)
	}

	cfg := DefaultConfig()
	cfg.ImagePath = imagePath
	srv := NewServer(cfg, nil, 0xFF, 0x10)
	msg := srv.advertise()
	if len(msg.Channels) != 4 {
		t.Fatalf("expected 4 channels, got %d", len(msg.Channels))
	}
	if msg.Channels[3].ID != srv.cfg.ImageChannelID {
		t.Fatalf("unexpected image channel id: %d", msg.Channels[3].ID)
	}
	if msg.Channels[3].Topic != srv.cfg.ImageTopic {
		t.Fatalf("unexpected image topic: %s", msg.Channels[3].Topic)
	}
}

func TestCompressedImageUsesBase64Payload(t *testing.T) {
	dir := t.TempDir()
	raw := []byte{1, 2, 3, 4, 5}
	imagePath := filepath.Join(dir, "demo.jpg")
	if err := os.WriteFile(imagePath, raw, 0o600); err != nil {
		t.Fatalf("write image: %v", err)
	}

	cfg := DefaultConfig()
	cfg.ImagePath = imagePath
	srv := NewServer(cfg, nil, 0xFF, 0x10)
	ts := time.Unix(123, 456)
	msg, ok := srv.compressedImage(ts)
	if !ok {
		t.Fatalf("expected compressed image")
	}
	if msg.FrameID != cfg.ImageFrameID {
		t.Fatalf("unexpected frame id: %s", msg.FrameID)
	}
	if msg.Format != cfg.ImageFormat {
		t.Fatalf("unexpected format: %s", msg.Format)
	}
	if msg.Data != base64.StdEncoding.EncodeToString(raw) {
		t.Fatalf("unexpected image payload: %s", msg.Data)
	}
}

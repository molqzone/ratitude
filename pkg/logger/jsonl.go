package logger

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"io"
	"time"

	"ratitude/pkg/protocol"
)

type JSONLWriter struct {
	enc    *json.Encoder
	textID uint8
}

type jsonRecord struct {
	TS         string `json:"ts"`
	ID         string `json:"id"`
	PayloadHex string `json:"payload_hex"`
	Data       any    `json:"data,omitempty"`
	Text       string `json:"text,omitempty"`
}

func NewJSONLWriter(w io.Writer, textID uint8) *JSONLWriter {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	return &JSONLWriter{
		enc:    enc,
		textID: textID,
	}
}

func (j *JSONLWriter) Consume(ctx context.Context, in <-chan protocol.RatPacket) {
	for {
		select {
		case <-ctx.Done():
			return
		case pkt, ok := <-in:
			if !ok {
				return
			}
			rec := jsonRecord{
				TS:         pkt.Timestamp.UTC().Format(time.RFC3339Nano),
				ID:         formatID(pkt.ID),
				PayloadHex: hex.EncodeToString(pkt.Payload),
				Data:       pkt.Data,
			}
			if pkt.ID == j.textID {
				if text, ok := pkt.Data.(string); ok {
					rec.Text = text
				}
			}
			_ = j.enc.Encode(rec)
		}
	}
}

func formatID(id uint8) string {
	return "0x" + hex.EncodeToString([]byte{id})
}

package bridge_test

import (
	"encoding/binary"
	"encoding/json"
	"strings"
	"testing"

	"ratitude/pkg/bridge/foxglove"
)

func TestEncodeMessageData(t *testing.T) {
	payload := []byte{0xAA, 0xBB}
	subID := uint32(7)
	logTime := uint64(0x1122334455667788)

	frame := foxglove.EncodeMessageData(subID, logTime, payload)
	if len(frame) != 1+4+8+len(payload) {
		t.Fatalf("unexpected frame length: %d", len(frame))
	}
	if frame[0] != foxglove.BinaryOpMessageData {
		t.Fatalf("unexpected opcode: 0x%02x", frame[0])
	}
	gotSub := binary.LittleEndian.Uint32(frame[1:5])
	if gotSub != subID {
		t.Fatalf("unexpected subscription id: %d", gotSub)
	}
	gotTime := binary.LittleEndian.Uint64(frame[5:13])
	if gotTime != logTime {
		t.Fatalf("unexpected log time: %d", gotTime)
	}
	if frame[13] != payload[0] || frame[14] != payload[1] {
		t.Fatalf("unexpected payload bytes: %v", frame[13:])
	}
}

func TestSubscribeUnsubscribeJSON(t *testing.T) {
	subData := []byte(`{"op":"subscribe","subscriptions":[{"id":1,"channelId":2}]}`)
	var sub foxglove.SubscribeMsg
	if err := json.Unmarshal(subData, &sub); err != nil {
		t.Fatalf("subscribe unmarshal failed: %v", err)
	}
	if sub.Op != foxglove.OpSubscribe || len(sub.Subscriptions) != 1 {
		t.Fatalf("unexpected subscribe payload: %+v", sub)
	}

	unsubData := []byte(`{"op":"unsubscribe","subscriptionIds":[1,2]}`)
	var unsub foxglove.UnsubscribeMsg
	if err := json.Unmarshal(unsubData, &unsub); err != nil {
		t.Fatalf("unsubscribe unmarshal failed: %v", err)
	}
	if unsub.Op != foxglove.OpUnsubscribe || len(unsub.SubscriptionIDs) != 2 {
		t.Fatalf("unexpected unsubscribe payload: %+v", unsub)
	}
}

func TestDefaultSchemaObject(t *testing.T) {
	schema := foxglove.DefaultSchema
	if strings.Contains(schema, "\"$ref\"") {
		t.Fatalf("schema should not contain $ref")
	}

	var parsed map[string]any
	if err := json.Unmarshal([]byte(schema), &parsed); err != nil {
		t.Fatalf("schema should be valid json: %v", err)
	}
	if parsed["type"] != "object" {
		t.Fatalf("expected schema type object, got %v", parsed["type"])
	}
}

func TestDefaultMarkerSchemaObject(t *testing.T) {
	schema := foxglove.DefaultMarkerSchema
	if strings.Contains(schema, "\"$ref\"") {
		t.Fatalf("marker schema should not contain $ref")
	}

	var parsed map[string]any
	if err := json.Unmarshal([]byte(schema), &parsed); err != nil {
		t.Fatalf("marker schema should be valid json: %v", err)
	}
	if parsed["type"] != "object" {
		t.Fatalf("expected marker schema type object, got %v", parsed["type"])
	}
}

func TestDefaultFrameTransformSchemaObject(t *testing.T) {
	schema := foxglove.DefaultFrameTransformSchema
	if strings.Contains(schema, "\"$ref\"") {
		t.Fatalf("transform schema should not contain $ref")
	}

	var parsed map[string]any
	if err := json.Unmarshal([]byte(schema), &parsed); err != nil {
		t.Fatalf("transform schema should be valid json: %v", err)
	}
	if parsed["type"] != "object" {
		t.Fatalf("expected transform schema type object, got %v", parsed["type"])
	}
}

func TestDefaultCompressedImageSchemaObject(t *testing.T) {
	schema := foxglove.DefaultCompressedImageSchema
	if strings.Contains(schema, "\"$ref\"") {
		t.Fatalf("compressed image schema should not contain $ref")
	}

	var parsed map[string]any
	if err := json.Unmarshal([]byte(schema), &parsed); err != nil {
		t.Fatalf("compressed image schema should be valid json: %v", err)
	}
	if parsed["type"] != "object" {
		t.Fatalf("expected compressed image schema type object, got %v", parsed["type"])
	}
}

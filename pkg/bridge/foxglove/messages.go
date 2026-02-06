package foxglove

import "encoding/binary"

const (
	OpServerInfo  = "serverInfo"
	OpAdvertise   = "advertise"
	OpSubscribe   = "subscribe"
	OpUnsubscribe = "unsubscribe"

	BinaryOpMessageData = 0x01
)

type ServerInfoMsg struct {
	Op                 string            `json:"op"`
	Name               string            `json:"name"`
	Capabilities       []string          `json:"capabilities"`
	SupportedEncodings []string          `json:"supportedEncodings,omitempty"`
	Metadata           map[string]string `json:"metadata,omitempty"`
	SessionID          string            `json:"sessionId,omitempty"`
}

type Channel struct {
	ID             uint64 `json:"id"`
	Topic          string `json:"topic"`
	Encoding       string `json:"encoding"`
	SchemaName     string `json:"schemaName"`
	SchemaEncoding string `json:"schemaEncoding,omitempty"`
	Schema         string `json:"schema,omitempty"`
}

type AdvertiseMsg struct {
	Op       string    `json:"op"`
	Channels []Channel `json:"channels"`
}

type Subscription struct {
	ID        uint32 `json:"id"`
	ChannelID uint64 `json:"channelId"`
}

type SubscribeMsg struct {
	Op            string         `json:"op"`
	Subscriptions []Subscription `json:"subscriptions"`
}

type UnsubscribeMsg struct {
	Op              string   `json:"op"`
	SubscriptionIDs []uint32 `json:"subscriptionIds"`
}

func EncodeMessageData(subscriptionID uint32, logTime uint64, payload []byte) []byte {
	out := make([]byte, 1+4+8+len(payload))
	out[0] = BinaryOpMessageData
	binary.LittleEndian.PutUint32(out[1:5], subscriptionID)
	binary.LittleEndian.PutUint64(out[5:13], logTime)
	copy(out[13:], payload)
	return out
}

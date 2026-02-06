package foxglove

const DefaultSchema = `{
  "type": "object",
  "properties": {
    "id": { "type": "string" },
    "ts": { "type": "string" },
    "payload_hex": { "type": "string" },
    "data": { "type": "object", "additionalProperties": true },
    "text": { "type": "string" }
  },
  "required": ["id", "payload_hex"]
}`

type Config struct {
	WSAddr         string
	Name           string
	Topic          string
	ChannelID      uint64
	SchemaName     string
	SchemaEncoding string
	Schema         string
	Encoding       string
	SendBuf        int
}

func DefaultConfig() Config {
	return Config{
		WSAddr:         "127.0.0.1:8765",
		Name:           "ratitude",
		Topic:          "ratitude/packet",
		ChannelID:      1,
		SchemaName:     "ratitude.Packet",
		SchemaEncoding: "jsonschema",
		Schema:         DefaultSchema,
		Encoding:       "json",
		SendBuf:        256,
	}
}

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

const DefaultMarkerSchema = `{
  "type": "object",
  "properties": {
    "header": {
      "type": "object",
      "properties": {
        "frame_id": { "type": "string" },
        "stamp": {
          "type": "object",
          "properties": {
            "sec": { "type": "integer" },
            "nsec": { "type": "integer" }
          },
          "required": ["sec", "nsec"]
        }
      },
      "required": ["frame_id", "stamp"]
    },
    "ns": { "type": "string" },
    "id": { "type": "integer" },
    "type": { "type": "integer" },
    "action": { "type": "integer" },
    "pose": {
      "type": "object",
      "properties": {
        "position": {
          "type": "object",
          "properties": {
            "x": { "type": "number" },
            "y": { "type": "number" },
            "z": { "type": "number" }
          },
          "required": ["x", "y", "z"]
        },
        "orientation": {
          "type": "object",
          "properties": {
            "x": { "type": "number" },
            "y": { "type": "number" },
            "z": { "type": "number" },
            "w": { "type": "number" }
          },
          "required": ["x", "y", "z", "w"]
        }
      },
      "required": ["position", "orientation"]
    },
    "scale": {
      "type": "object",
      "properties": {
        "x": { "type": "number" },
        "y": { "type": "number" },
        "z": { "type": "number" }
      },
      "required": ["x", "y", "z"]
    },
    "color": {
      "type": "object",
      "properties": {
        "r": { "type": "number" },
        "g": { "type": "number" },
        "b": { "type": "number" },
        "a": { "type": "number" }
      },
      "required": ["r", "g", "b", "a"]
    }
  },
  "required": ["header", "ns", "id", "type", "action", "pose", "scale", "color"]
}`

const DefaultFrameTransformSchema = `{
  "type": "object",
  "properties": {
    "transforms": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "timestamp": {
            "type": "object",
            "properties": {
              "sec": { "type": "integer" },
              "nsec": { "type": "integer" }
            },
            "required": ["sec", "nsec"]
          },
          "parent_frame_id": { "type": "string" },
          "child_frame_id": { "type": "string" },
          "translation": {
            "type": "object",
            "properties": {
              "x": { "type": "number" },
              "y": { "type": "number" },
              "z": { "type": "number" }
            },
            "required": ["x", "y", "z"]
          },
          "rotation": {
            "type": "object",
            "properties": {
              "x": { "type": "number" },
              "y": { "type": "number" },
              "z": { "type": "number" },
              "w": { "type": "number" }
            },
            "required": ["x", "y", "z", "w"]
          }
        },
        "required": ["timestamp", "parent_frame_id", "child_frame_id", "translation", "rotation"]
      }
    }
  },
  "required": ["transforms"]
}`

const DefaultCompressedImageSchema = `{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "frame_id": { "type": "string" },
    "format": { "type": "string" },
    "data": { "type": "string", "contentEncoding": "base64" }
  },
  "required": ["timestamp", "frame_id", "format", "data"]
}`

const DefaultLogSchema = `{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "level": { "type": "integer" },
    "message": { "type": "string" },
    "name": { "type": "string" },
    "file": { "type": "string" },
    "line": { "type": "integer" }
  },
  "required": ["timestamp", "level", "message", "name", "file", "line"]
}`

const DefaultTemperatureSchema = `{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "value": { "type": "number" },
    "unit": { "type": "string" }
  },
  "required": ["timestamp", "value", "unit"]
}`

type Config struct {
	WSAddr                  string
	Name                    string
	Topic                   string
	ChannelID               uint64
	SchemaName              string
	SchemaEncoding          string
	Schema                  string
	Encoding                string
	MarkerTopic             string
	MarkerChannelID         uint64
	MarkerSchemaName        string
	MarkerSchemaEncoding    string
	MarkerSchema            string
	MarkerEncoding          string
	TransformTopic          string
	TransformChannelID      uint64
	TransformSchemaName     string
	TransformSchemaEncoding string
	TransformSchema         string
	TransformEncoding       string
	ImageTopic              string
	ImageChannelID          uint64
	ImageSchemaName         string
	ImageSchemaEncoding     string
	ImageSchema             string
	ImageEncoding           string
	ImagePath               string
	ImageFrameID            string
	ImageFormat             string
	LogTopic                string
	LogChannelID            uint64
	LogSchemaName           string
	LogSchemaEncoding       string
	LogSchema               string
	LogEncoding             string
	LogName                 string
	TempTopic               string
	TempChannelID           uint64
	TempSchemaName          string
	TempSchemaEncoding      string
	TempSchema              string
	TempEncoding            string
	TempUnit                string
	ParentFrameID           string
	FrameID                 string
	SendBuf                 int
}

func DefaultConfig() Config {
	return Config{
		WSAddr:                  "127.0.0.1:8765",
		Name:                    "ratitude",
		Topic:                   "ratitude/packet",
		ChannelID:               1,
		SchemaName:              "ratitude.Packet",
		SchemaEncoding:          "jsonschema",
		Schema:                  DefaultSchema,
		Encoding:                "json",
		MarkerTopic:             "/visualization_marker",
		MarkerChannelID:         2,
		MarkerSchemaName:        "visualization_msgs/Marker",
		MarkerSchemaEncoding:    "jsonschema",
		MarkerSchema:            DefaultMarkerSchema,
		MarkerEncoding:          "json",
		TransformTopic:          "/tf",
		TransformChannelID:      3,
		TransformSchemaName:     "foxglove.FrameTransforms",
		TransformSchemaEncoding: "jsonschema",
		TransformSchema:         DefaultFrameTransformSchema,
		TransformEncoding:       "json",
		ImageTopic:              "/camera/image/compressed",
		ImageChannelID:          4,
		ImageSchemaName:         "foxglove.CompressedImage",
		ImageSchemaEncoding:     "jsonschema",
		ImageSchema:             DefaultCompressedImageSchema,
		ImageEncoding:           "json",
		ImagePath:               "D:/Repos/ratitude/demo.jpg",
		ImageFrameID:            "camera",
		ImageFormat:             "jpeg",
		LogTopic:                "/ratitude/log",
		LogChannelID:            5,
		LogSchemaName:           "foxglove.Log",
		LogSchemaEncoding:       "jsonschema",
		LogSchema:               DefaultLogSchema,
		LogEncoding:             "json",
		LogName:                 "ratitude",
		TempTopic:               "/ratitude/temperature",
		TempChannelID:           6,
		TempSchemaName:          "ratitude.Temperature",
		TempSchemaEncoding:      "jsonschema",
		TempSchema:              DefaultTemperatureSchema,
		TempEncoding:            "json",
		TempUnit:                "C",
		ParentFrameID:           "world",
		FrameID:                 "base_link",
		SendBuf:                 256,
	}
}

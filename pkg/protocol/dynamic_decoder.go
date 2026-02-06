package protocol

import (
	"encoding/binary"
	"fmt"
	"math"
	"strings"
	"sync"
)

type DynamicFieldDef struct {
	Name   string
	CType  string
	Offset int
	Size   int
}

type DynamicPacketDef struct {
	ID         uint8
	StructName string
	Packed     bool
	ByteSize   int
	Fields     []DynamicFieldDef
}

var (
	dynamicRegistryMu sync.RWMutex
	dynamicRegistry   = map[uint8]DynamicPacketDef{}
)

func ClearDynamicRegistry() {
	dynamicRegistryMu.Lock()
	dynamicRegistry = map[uint8]DynamicPacketDef{}
	dynamicRegistryMu.Unlock()
}

func RegisterDynamic(id uint8, def DynamicPacketDef) error {
	if def.ByteSize <= 0 {
		return fmt.Errorf("invalid byte size: %d", def.ByteSize)
	}
	if len(def.Fields) == 0 {
		return fmt.Errorf("dynamic packet requires at least one field")
	}

	normalized := DynamicPacketDef{
		ID:         id,
		StructName: def.StructName,
		Packed:     def.Packed,
		ByteSize:   def.ByteSize,
		Fields:     make([]DynamicFieldDef, 0, len(def.Fields)),
	}

	for _, field := range def.Fields {
		ctype := normalizeDynamicType(field.CType)
		size, ok := dynamicTypeSize(ctype)
		if !ok {
			return fmt.Errorf("unsupported c type %q", field.CType)
		}
		if field.Size != size {
			return fmt.Errorf("field %s size mismatch: got %d want %d", field.Name, field.Size, size)
		}
		if field.Offset < 0 {
			return fmt.Errorf("field %s has invalid offset %d", field.Name, field.Offset)
		}
		if field.Offset+field.Size > def.ByteSize {
			return fmt.Errorf("field %s exceeds packet size", field.Name)
		}
		normalized.Fields = append(normalized.Fields, DynamicFieldDef{
			Name:   field.Name,
			CType:  ctype,
			Offset: field.Offset,
			Size:   field.Size,
		})
	}

	dynamicRegistryMu.Lock()
	dynamicRegistry[id] = normalized
	dynamicRegistryMu.Unlock()
	return nil
}

func parseDynamicPacket(id uint8, payload []byte) (map[string]any, bool, error) {
	dynamicRegistryMu.RLock()
	def, ok := dynamicRegistry[id]
	dynamicRegistryMu.RUnlock()
	if !ok {
		return nil, false, nil
	}

	if len(payload) != def.ByteSize {
		return nil, true, fmt.Errorf("payload size %d does not match dynamic packet size %d for id 0x%02x", len(payload), def.ByteSize, id)
	}

	out := make(map[string]any, len(def.Fields))
	for _, field := range def.Fields {
		start := field.Offset
		end := start + field.Size
		if end > len(payload) {
			return nil, true, fmt.Errorf("field %s out of range for id 0x%02x", field.Name, id)
		}
		value, err := decodeDynamicValue(field.CType, payload[start:end])
		if err != nil {
			return nil, true, fmt.Errorf("decode field %s for id 0x%02x: %w", field.Name, id, err)
		}
		out[field.Name] = value
	}

	return out, true, nil
}

func decodeDynamicValue(ctype string, data []byte) (any, error) {
	switch ctype {
	case "float":
		return math.Float32frombits(binary.LittleEndian.Uint32(data)), nil
	case "double":
		return math.Float64frombits(binary.LittleEndian.Uint64(data)), nil
	case "int8_t":
		return int8(data[0]), nil
	case "uint8_t":
		return uint8(data[0]), nil
	case "int16_t":
		return int16(binary.LittleEndian.Uint16(data)), nil
	case "uint16_t":
		return binary.LittleEndian.Uint16(data), nil
	case "int32_t":
		return int32(binary.LittleEndian.Uint32(data)), nil
	case "uint32_t":
		return binary.LittleEndian.Uint32(data), nil
	case "int64_t":
		return int64(binary.LittleEndian.Uint64(data)), nil
	case "uint64_t":
		return binary.LittleEndian.Uint64(data), nil
	case "bool", "_bool":
		return data[0] != 0, nil
	default:
		return nil, fmt.Errorf("unsupported c type %q", ctype)
	}
}

func dynamicTypeSize(ctype string) (int, bool) {
	switch ctype {
	case "float":
		return 4, true
	case "double":
		return 8, true
	case "int8_t", "uint8_t", "bool", "_bool":
		return 1, true
	case "int16_t", "uint16_t":
		return 2, true
	case "int32_t", "uint32_t":
		return 4, true
	case "int64_t", "uint64_t":
		return 8, true
	default:
		return 0, false
	}
}

func normalizeDynamicType(raw string) string {
	s := strings.ToLower(strings.TrimSpace(raw))
	s = strings.ReplaceAll(s, "\t", " ")
	for strings.Contains(s, "  ") {
		s = strings.ReplaceAll(s, "  ", " ")
	}
	s = strings.TrimPrefix(s, "const ")
	s = strings.TrimPrefix(s, "volatile ")
	return strings.TrimSpace(s)
}

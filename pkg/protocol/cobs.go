package protocol

import "fmt"

// CobsDecode decodes a COBS frame without the trailing 0x00 delimiter.
func CobsDecode(frame []byte) ([]byte, error) {
	if len(frame) == 0 {
		return nil, nil
	}

	out := make([]byte, 0, len(frame))
	for i := 0; i < len(frame); {
		code := frame[i]
		if code == 0 {
			return nil, fmt.Errorf("invalid COBS code 0x00")
		}
		i++

		count := int(code) - 1
		if i+count > len(frame) {
			return nil, fmt.Errorf("cobs frame truncated")
		}

		out = append(out, frame[i:i+count]...)
		i += count

		if code != 0xFF && i < len(frame) {
			out = append(out, 0x00)
		}
	}

	return out, nil
}

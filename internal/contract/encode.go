package contract

import (
	"encoding/json"
	"strconv"
	"strings"
)

// EncodeJSON encodes a contract value with serde_json's string escaping
// and scalar floating-point spelling.
// Call this or generated MarshalJSON methods directly for wire bytes: wrapping
// them in encoding/json.Marshal reapplies Go's HTML and JavaScript escaping.
func EncodeJSON(value any) ([]byte, error) {
	data, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	switch number := value.(type) {
	case float32:
		return formatJSONFloat(float64(number), 32)
	case float64:
		return formatJSONFloat(number, 64)
	}
	out := make([]byte, 0, len(data))
	for i := 0; i < len(data); {
		if data[i] != '"' {
			out = append(out, data[i])
			i++
			continue
		}
		start := i
		i++
		for data[i] != '"' {
			if data[i] == '\\' {
				i++
			}
			i++
		}
		i++
		var text string
		if err := json.Unmarshal(data[start:i], &text); err != nil {
			return nil, err
		}
		out = appendJSONString(out, text)
	}
	return out, nil
}

// appendJSONString writes contract strings using serde_json's ESCAPE table:
// only quotes, backslashes and ASCII controls need escaping. The standard
// library's JSON encoder also escapes U+2028/U+2029 with HTML escaping disabled.
func appendJSONString(out []byte, text string) []byte {
	const hex = "0123456789abcdef"
	out = append(out, '"')
	for i := 0; i < len(text); i++ {
		c := text[i]
		switch c {
		case '"', '\\':
			out = append(out, '\\', c)
		case '\b':
			out = append(out, '\\', 'b')
		case '\t':
			out = append(out, '\\', 't')
		case '\n':
			out = append(out, '\\', 'n')
		case '\f':
			out = append(out, '\\', 'f')
		case '\r':
			out = append(out, '\\', 'r')
		default:
			if c < 0x20 {
				out = append(out, '\\', 'u', '0', '0', hex[c>>4], hex[c&15])
			} else {
				out = append(out, c)
			}
		}
	}
	return append(out, '"')
}

// formatJSONFloat retains serde_json's float kind, negative zero and exponent
// spelling. EncodeJSON has already rejected non-finite values.
func formatJSONFloat(number float64, bits int) ([]byte, error) {
	text := strconv.FormatFloat(number, 'e', -1, bits)
	mantissa, exponent, _ := strings.Cut(text, "e")
	power, err := strconv.Atoi(exponent)
	if err != nil {
		return nil, err
	}
	// serde_json's zmij formatter uses different fixed-notation intervals
	// for f32 and f64 (zmij 1.0.23, FIXED_DEC_EXP).
	minimum, maximum := -5, 15
	if bits == 32 {
		minimum, maximum = -6, 12
	}
	if power >= minimum && power <= maximum {
		text = strconv.FormatFloat(number, 'f', -1, bits)
		if !strings.Contains(text, ".") {
			text += ".0"
		}
	} else {
		sign := ""
		if power >= 0 {
			sign = "+"
		}
		text = mantissa + "e" + sign + strconv.Itoa(power)
	}
	return []byte(text), nil
}

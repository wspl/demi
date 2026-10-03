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
	output := make([]byte, 0, len(data))
	for i := 0; i < len(data); {
		if data[i] != '"' {
			output = append(output, data[i])
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
		output = appendJSONString(output, text)
	}
	return output, nil
}

// appendJSONString writes contract strings using serde_json's ESCAPE table:
// only quotes, backslashes and ASCII controls need escaping. The standard
// library's JSON encoder also escapes U+2028/U+2029 with HTML escaping disabled.
func appendJSONString(output []byte, text string) []byte {
	const hex = "0123456789abcdef"
	output = append(output, '"')
	for i := 0; i < len(text); i++ {
		c := text[i]
		switch c {
		case '"', '\\':
			output = append(output, '\\', c)
		case '\b':
			output = append(output, '\\', 'b')
		case '\t':
			output = append(output, '\\', 't')
		case '\n':
			output = append(output, '\\', 'n')
		case '\f':
			output = append(output, '\\', 'f')
		case '\r':
			output = append(output, '\\', 'r')
		default:
			if c < 0x20 {
				output = append(output, '\\', 'u', '0', '0', hex[c>>4], hex[c&15])
			} else {
				output = append(output, c)
			}
		}
	}
	return append(output, '"')
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

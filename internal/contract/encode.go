package contract

import "encoding/json"

// EncodeJSON encodes a contract value with serde_json's string escaping.
// Call this or generated MarshalJSON methods directly for wire bytes: wrapping
// them in encoding/json.Marshal reapplies Go's HTML and JavaScript escaping.
func EncodeJSON(value any) ([]byte, error) {
	data, err := json.Marshal(value)
	if err != nil {
		return nil, err
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

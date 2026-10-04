package contract

import (
	"encoding/json"
	"fmt"
	"reflect"
	"strconv"
	"strings"
)

// EncodeJSON encodes a contract value with the contract's string escaping
// (appendJSONString) and floating-point spelling (formatJSONFloat).
// Raw values normalize number spellings as parsed JSON values. Custom codecs
// retain their numeric representations; all strings use the same escaping.
// Call this or generated MarshalJSON methods directly for wire bytes: wrapping
// them in encoding/json.Marshal reapplies Go's HTML and JavaScript escaping.
func EncodeJSON(value any) ([]byte, error) {
	_, custom := value.(json.Marshaler)
	v := reflect.ValueOf(value)
	if !custom && v.IsValid() && (v.Kind() == reflect.Slice || v.Kind() == reflect.Array) &&
		v.Type().Elem().Kind() != reflect.Uint8 {
		return encodeJSONList(v)
	}
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
	_, raw := value.(json.RawMessage)
	_, rawPointer := value.(*json.RawMessage)
	return normalizeJSON(data, raw || rawPointer)
}

// normalizeJSON writes compact, normalized tokens without sorting object members.
// Its input has already passed through the standard encoder's syntax checks.
// Only raw values normalize numbers: a generated codec may encode float32,
// whose exponent thresholds differ from those of a parsed JSON float64.
func normalizeJSON(data []byte, numbers bool) ([]byte, error) {
	output := make([]byte, 0, len(data))
	for i := 0; i < len(data); {
		if numbers && (data[i] == '-' || data[i] >= '0' && data[i] <= '9') {
			end := i + 1
			for end < len(data) && strings.ContainsRune("0123456789.eE+-", rune(data[end])) {
				end++
			}
			number, err := normalizeJSONNumber(string(data[i:end]))
			if err != nil {
				return nil, err
			}
			output = append(output, number...)
			i = end
			continue
		}
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

// appendJSONString escapes only quotes, backslashes and ASCII controls
// (\b \t \n \f \r by name, the others as \u00XX) and writes every other byte as is. The standard
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

// formatJSONFloat spells a float as a float: fixed notation with at least one
// decimal (1.0, -0.0) while the decimal exponent is within -5..15 for float64
// or -6..12 for float32, otherwise the shortest mantissa and a signed exponent
// (1e-7, 1e+16). EncodeJSON has already rejected non-finite values.
func formatJSONFloat(number float64, bits int) ([]byte, error) {
	text := strconv.FormatFloat(number, 'e', -1, bits)
	mantissa, exponent, _ := strings.Cut(text, "e")
	power, err := strconv.Atoi(exponent)
	if err != nil {
		return nil, err
	}
	// float32 keeps fixed notation over a narrower exponent range than float64.
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

// encodeJSONList applies the scalar number spelling to collection elements.
func encodeJSONList(value reflect.Value) ([]byte, error) {
	// Keep the standard encoder's cycle and unsupported-value checks before
	// traversing elements to normalize their numeric representations.
	if _, err := json.Marshal(value.Interface()); err != nil {
		return nil, err
	}
	if value.Kind() == reflect.Slice && value.IsNil() {
		return []byte("null"), nil
	}
	output := []byte{'['}
	for i := 0; i < value.Len(); i++ {
		item, err := EncodeJSON(value.Index(i).Interface())
		if err != nil {
			return nil, At("["+strconv.Itoa(i)+"]", err)
		}
		if i > 0 {
			output = append(output, ',')
		}
		output = append(output, item...)
	}
	return append(output, ']'), nil
}

// normalizeJSONNumber retains signed and unsigned integer precision and spells
// decimal, exponent, negative-zero and larger integer tokens as finite floats.
func normalizeJSONNumber(text string) ([]byte, error) {
	if !strings.ContainsAny(text, ".eE") && text != "-0" {
		if _, err := strconv.ParseInt(text, 10, 64); err == nil {
			return []byte(text), nil
		}
		if _, err := strconv.ParseUint(text, 10, 64); err == nil {
			return []byte(text), nil
		}
	}
	number, err := strconv.ParseFloat(text, 64)
	if err != nil {
		return nil, fmt.Errorf("decode JSON number: %w", err)
	}
	return formatJSONFloat(number, 64)
}

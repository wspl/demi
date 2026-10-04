package contract

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"unicode"
)

// JSONObject retains an object and its member order without rewriting its bytes.
func JSONObject(data []byte) (json.RawMessage, error) {
	if err := CheckJSON(data); err != nil {
		return nil, err
	}
	trimmed := bytes.TrimSpace(data)
	if trimmed[0] == '{' {
		if _, err := ObjectFields(data); err != nil {
			return nil, err
		}
		return bytes.Clone(data), nil
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	token, err := decoder.Token()
	if err != nil {
		return nil, err
	}
	unexpected := "sequence"
	switch value := token.(type) {
	case nil:
		unexpected = "null"
	case string:
		unexpected = "string " + objectErrorString(value)
	case bool:
		unexpected = fmt.Sprintf("boolean `%t`", value)
	case json.Number:
		unexpected, err = objectErrorNumber(value)
		if err != nil {
			return nil, err
		}
	}
	return nil, fmt.Errorf("invalid type: %s, expected a map", unexpected)
}

// OrderedObject encodes object bytes verbatim after checking their shape.
// Generated encoders use this wrapper while fields remain json.RawMessage.
type OrderedObject json.RawMessage

// MarshalJSON returns the validated object without rewriting member bytes.
func (v OrderedObject) MarshalJSON() ([]byte, error) {
	return JSONObject(v)
}

// objectErrorNumber spells a refused numeric token in the decoder's diagnostic format.
func objectErrorNumber(value json.Number) (string, error) {
	text := string(value)
	if !strings.ContainsAny(text, ".eE") && text != "-0" {
		if _, err := strconv.ParseInt(text, 10, 64); err == nil {
			return "integer `" + text + "`", nil
		}
		if _, err := strconv.ParseUint(text, 10, 64); err == nil {
			return "integer `" + text + "`", nil
		}
	}
	number, err := value.Float64()
	if err != nil {
		return "", fmt.Errorf("decode refused number: %w", err)
	}
	encoded, err := formatJSONFloat(number, 64)
	if err != nil {
		return "", fmt.Errorf("format refused number: %w", err)
	}
	return "floating point `" + string(encoded) + "`", nil
}

// objectErrorString uses the decoder's quoted diagnostic spelling, including
// brace-delimited Unicode escapes for non-printing characters.
func objectErrorString(value string) string {
	var out strings.Builder
	out.WriteByte('"')
	for _, char := range value {
		switch char {
		case 0:
			out.WriteString(`\0`)
		case '\t':
			out.WriteString(`\t`)
		case '\r':
			out.WriteString(`\r`)
		case '\n':
			out.WriteString(`\n`)
		case '\\', '"':
			out.WriteByte('\\')
			out.WriteRune(char)
		default:
			if unicode.IsPrint(char) {
				out.WriteRune(char)
			} else {
				out.WriteString(`\u{` + strconv.FormatInt(int64(char), 16) + `}`)
			}
		}
	}
	out.WriteByte('"')
	return out.String()
}

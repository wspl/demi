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

// objectErrorNumber spells a refused numeric token in the decoder's diagnostic format.
func objectErrorNumber(value json.Number) (string, error) {
	encoded, err := normalizeJSONNumber(string(value))
	if err != nil {
		return "", err
	}
	kind := "integer"
	if bytes.ContainsAny(encoded, ".eE") {
		kind = "floating point"
	}
	return kind + " `" + string(encoded) + "`", nil
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

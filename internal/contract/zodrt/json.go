package zodrt

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"maps"
	"math/big"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"
)

// The markers of `stringifyPortableJson` in @demicodes/utils, which carries
// the values plain JSON cannot.
const (
	bytesMarker  = "__demiUint8Array"
	bigIntMarker = "__demiBigInt"
	dateMarker   = "__demiDate"
)

// DecodeJSON decodes JSON text into a tree, as `JSON.parse` does. A portable
// document also revives the marked bytes, big integers and dates of
// `parsePortableJson`.
func DecodeJSON(data []byte, portable bool) (any, error) {
	var value any
	if err := json.Unmarshal(data, &value); err != nil {
		return nil, fmt.Errorf("malformed JSON: %w", err)
	}
	if !portable {
		return value, nil
	}
	return revive(value)
}

// revive replaces portable markers bottom-up, as a JSON.parse reviver does.
func revive(value any) (any, error) {
	switch node := value.(type) {
	case []any:
		for index, item := range node {
			revived, err := revive(item)
			if err != nil {
				return nil, err
			}
			node[index] = revived
		}
		return node, nil
	case map[string]any:
		for key, item := range node {
			revived, err := revive(item)
			if err != nil {
				return nil, err
			}
			node[key] = revived
		}
		return reviveMarker(node)
	}
	return value, nil
}

func reviveMarker(node map[string]any) (any, error) {
	if node[bytesMarker] == true {
		if text, ok := node["base64"].(string); ok {
			data, err := base64.StdEncoding.DecodeString(strings.Join(strings.Fields(text), ""))
			if err != nil {
				return nil, fmt.Errorf("malformed portable bytes: %w", err)
			}
			return data, nil
		}
	}
	if node[bigIntMarker] == true {
		if text, ok := node["value"].(string); ok {
			number, ok := new(big.Int).SetString(text, 10)
			if !ok {
				return nil, fmt.Errorf("malformed portable big integer %q", text)
			}
			return number, nil
		}
	}
	if node[dateMarker] == true {
		if text, ok := node["iso"].(string); ok {
			return parseISOString(text)
		}
	}
	return node, nil
}

// EncodeJSON encodes a tree as `JSON.stringify` does: objects in field order,
// numbers and strings in JavaScript's notation. Bytes, big integers and dates
// need a portable document, which marks them as `stringifyPortableJson` does.
// A value of another Go type is encoded by encoding/json.
func EncodeJSON(value any, portable bool) ([]byte, error) {
	var out bytes.Buffer
	if err := writeJSON(&out, value, portable); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

func writeJSON(out *bytes.Buffer, value any, portable bool) error {
	switch node := value.(type) {
	case nil:
		out.WriteString("null")
	case bool:
		out.WriteString(strconv.FormatBool(node))
	case float64:
		return writeJSONNumber(out, node)
	case int64:
		out.WriteString(strconv.FormatInt(node, 10))
	case string:
		writeJSONString(out, node)
	case []byte:
		if !portable {
			return fmt.Errorf("plain JSON cannot carry bytes")
		}
		return writeJSON(out, Object{{Key: bytesMarker, Value: true}, {Key: "base64", Value: base64.StdEncoding.EncodeToString(node)}}, false)
	case *big.Int:
		if !portable {
			return fmt.Errorf("plain JSON cannot carry a big integer")
		}
		return writeJSON(out, Object{{Key: bigIntMarker, Value: true}, {Key: "value", Value: node.String()}}, false)
	case time.Time:
		if !portable {
			return fmt.Errorf("plain JSON cannot carry a date")
		}
		return writeJSON(out, Object{{Key: dateMarker, Value: true}, {Key: "iso", Value: isoString(node)}}, false)
	case []any:
		out.WriteByte('[')
		for index, item := range node {
			if index > 0 {
				out.WriteByte(',')
			}
			if err := writeJSON(out, item, portable); err != nil {
				return err
			}
		}
		out.WriteByte(']')
	case Object:
		out.WriteByte('{')
		for index, field := range node {
			if index > 0 {
				out.WriteByte(',')
			}
			writeJSONString(out, field.Key)
			out.WriteByte(':')
			if err := writeJSON(out, field.Value, portable); err != nil {
				return err
			}
		}
		out.WriteByte('}')
	case map[string]any:
		// A Go map has no order; its keys are written sorted.
		fields := make(Object, 0, len(node))
		for _, key := range slices.Sorted(maps.Keys(node)) {
			fields = append(fields, Field{Key: key, Value: node[key]})
		}
		return writeJSON(out, fields, portable)
	default:
		encoder := json.NewEncoder(out)
		encoder.SetEscapeHTML(false)
		if err := encoder.Encode(node); err != nil {
			return err
		}
		// json.Encoder ends each value with a newline.
		out.Truncate(out.Len() - 1)
	}
	return nil
}

// writeJSONNumber writes a number as JSON.stringify does. encoding/json
// already formats float64 the way ECMAScript's Number::toString does; negative
// zero is the one difference.
func writeJSONNumber(out *bytes.Buffer, number float64) error {
	if number == 0 {
		out.WriteByte('0')
		return nil
	}
	text, err := json.Marshal(number)
	if err != nil {
		return err
	}
	out.Write(text)
	return nil
}

// writeJSONString quotes a string as JSON.stringify does. encoding/json also
// escapes U+2028, U+2029 and, by default, HTML characters, which JavaScript
// writes as they are.
func writeJSONString(out *bytes.Buffer, text string) {
	out.WriteByte('"')
	for _, char := range text {
		switch char {
		case '"':
			out.WriteString(`\"`)
		case '\\':
			out.WriteString(`\\`)
		case '\b':
			out.WriteString(`\b`)
		case '\f':
			out.WriteString(`\f`)
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case '\t':
			out.WriteString(`\t`)
		default:
			if char < 0x20 {
				fmt.Fprintf(out, `\u%04x`, char)
			} else {
				out.WriteRune(char)
			}
		}
	}
	out.WriteByte('"')
}

// isoString formats a time as Date.prototype.toISOString does.
func isoString(instant time.Time) string {
	instant = instant.UTC()
	year := instant.Year()
	rest := instant.Format("-01-02T15:04:05.000Z")
	if year >= 0 && year <= 9999 {
		return fmt.Sprintf("%04d%s", year, rest)
	}
	if year < 0 {
		return fmt.Sprintf("-%06d%s", -year, rest)
	}
	return fmt.Sprintf("+%06d%s", year, rest)
}

// isoPattern is the form Date.prototype.toISOString writes.
var isoPattern = regexp.MustCompile(`^([+-]\d{6}|\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{3})Z$`)

// parseISOString reads the form isoString writes, the only form
// stringifyPortableJson writes.
func parseISOString(text string) (time.Time, error) {
	match := isoPattern.FindStringSubmatch(text)
	if match == nil {
		return time.Time{}, fmt.Errorf("malformed portable date %q", text)
	}
	parts := make([]int, len(match)-1)
	for index, part := range match[1:] {
		// The pattern admits only digits and a sign.
		parts[index], _ = strconv.Atoi(part)
	}
	instant := time.Date(parts[0], time.Month(parts[1]), parts[2], parts[3], parts[4], parts[5], parts[6]*int(time.Millisecond), time.UTC)
	// time.Date normalizes an impossible date such as February 30; the
	// round trip rejects it.
	if isoString(instant) != text {
		return time.Time{}, fmt.Errorf("invalid portable date %q", text)
	}
	return instant, nil
}

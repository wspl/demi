package wire

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"io"
	"strconv"
	"strings"

	"github.com/tinylib/msgp/msgp"
)

// MPJSON reads a JSON-valued MessagePack subtree without repairing binary or extensions.
func MPJSON(data []byte) (jsontext.Value, error) {
	raw, err := mpJSONValue(data, 0)
	if err != nil {
		return nil, err
	}
	value := jsontext.Value(raw)
	if !value.IsValid() {
		return nil, &InvalidError{Rule: "invalid JSON value"}
	}
	return value, nil
}

// mpJSONValue preserves opaque JSON object order and floating-point number kind.
// msgp's JSON writer renders integral floats as integers; adding .0 retains
// serde_json's float representation when this value is encoded again.
func mpJSONValue(data []byte, depth int) ([]byte, error) {
	if depth > 1000 {
		return nil, &InvalidError{Rule: "maximum nesting depth exceeded"}
	}
	kind := msgp.NextType(data)
	switch kind {
	case msgp.MapType:
		fields, err := MPObject(data)
		if err != nil {
			return nil, err
		}
		output := []byte{'{'}
		for i, field := range fields {
			if i > 0 {
				output = append(output, ',')
			}
			key, err := json.Marshal(field.Name)
			if err != nil {
				return nil, Refusal(err)
			}
			value, err := mpJSONValue(field.Data, depth+1)
			if err != nil {
				return nil, In(Key(field.Name), err)
			}
			output = append(output, key...)
			output = append(output, ':')
			output = append(output, value...)
		}
		return append(output, '}'), nil
	case msgp.ArrayType:
		items, err := MPArray(data, func(b []byte) ([]byte, error) { return mpJSONValue(b, depth+1) })
		if err != nil {
			return nil, err
		}
		output := []byte{'['}
		for i, item := range items {
			if i > 0 {
				output = append(output, ',')
			}
			output = append(output, item...)
		}
		return append(output, ']'), nil
	case msgp.StrType:
		if _, err := MPString(data); err != nil {
			return nil, err
		}
	case msgp.Float32Type, msgp.Float64Type:
		if _, err := MPFloat(data); err != nil {
			return nil, err
		}
	case msgp.NilType, msgp.BoolType, msgp.IntType, msgp.UintType:
	default:
		return nil, &InvalidError{Rule: "expected JSON value"}
	}
	var output bytes.Buffer
	rest, err := msgp.UnmarshalAsJSON(&output, data)
	if err != nil || len(rest) != 0 {
		return nil, &InvalidError{Rule: "invalid MessagePack JSON value"}
	}
	raw := output.Bytes()
	if (kind == msgp.Float32Type || kind == msgp.Float64Type) && !strings.ContainsAny(string(raw), ".eE") {
		raw = append(raw, '.', '0')
	}
	return raw, nil
}

// JSONMsgpack writes an opaque JSON value with insertion-ordered map keys, as serde_json with preserve_order does.
func JSONMsgpack(value jsontext.Value) ([]byte, error) {
	dec := jsontext.NewDecoder(bytes.NewReader(value))
	data, err := jsonMP(dec)
	if err != nil {
		return nil, Refusal(err)
	}
	if _, err := dec.ReadToken(); err != io.EOF {
		return nil, &InvalidError{Rule: "trailing JSON data"}
	}
	return data, nil
}

func jsonMP(dec *jsontext.Decoder) ([]byte, error) {
	token, err := dec.ReadToken()
	if err != nil {
		return nil, err
	}
	switch token.Kind() {
	case 'n':
		return msgp.AppendNil(nil), nil
	case 't', 'f':
		return msgp.AppendBool(nil, token.Bool()), nil
	case '"':
		return msgp.AppendString(nil, token.String()), nil
	case '0':
		text := token.String()
		if n, err := strconv.ParseUint(text, 10, 64); err == nil {
			return msgp.AppendUint64(nil, n), nil
		}
		if n, err := strconv.ParseInt(text, 10, 64); err == nil && text != "-0" {
			return msgp.AppendInt64(nil, n), nil
		}
		f, err := strconv.ParseFloat(text, 64)
		if err != nil {
			return nil, err
		}
		return msgp.AppendFloat64(nil, f), nil
	case '[':
		var items [][]byte
		for dec.PeekKind() != ']' {
			item, err := jsonMP(dec)
			if err != nil {
				return nil, err
			}
			items = append(items, item)
		}
		if _, err := dec.ReadToken(); err != nil {
			return nil, err
		}
		data := msgp.AppendArrayHeader(nil, uint32(len(items)))
		for _, item := range items {
			data = append(data, item...)
		}
		return data, nil
	case '{':
		var fields []MPMember
		for dec.PeekKind() != '}' {
			name, err := dec.ReadToken()
			if err != nil {
				return nil, err
			}
			key := name.String()
			item, err := jsonMP(dec)
			if err != nil {
				return nil, err
			}
			fields = append(fields, MPMember{Name: key, Data: item})
		}
		if _, err := dec.ReadToken(); err != nil {
			return nil, err
		}
		data := msgp.AppendMapHeader(nil, uint32(len(fields)))
		for _, field := range fields {
			data = msgp.AppendString(data, field.Name)
			data = append(data, field.Data...)
		}
		return data, nil
	}
	return nil, &InvalidError{Rule: "expected JSON value"}
}

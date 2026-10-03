package contract

import (
	"bytes"
	"encoding/json"
	"errors"
	"math"
	"reflect"
	"strconv"
	"strings"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/vmihailenco/msgpack/v5/msgpcode"
)

// encodeOpaqueMsgpack keeps each object's member order and each number's kind, integer or float.
func encodeOpaqueMsgpack(encoder *msgpack.Encoder, data json.RawMessage) error {
	if err := CheckJSON(data); err != nil {
		return err
	}
	data = bytes.TrimSpace(data)
	switch data[0] {
	case '{':
		fields, err := ObjectFields(data)
		if err != nil {
			return err
		}
		if err := encoder.EncodeMapLen(len(fields)); err != nil {
			return err
		}
		for _, field := range fields {
			if err := encoder.EncodeString(field.Name); err != nil {
				return err
			}
			if err := encodeMsgpackValue(encoder, reflect.ValueOf(field.Value)); err != nil {
				return At(field.Name, err)
			}
		}
		return nil
	case '[':
		values, err := Decode[[]json.RawMessage](data)
		if err != nil {
			return err
		}
		if err := encoder.EncodeArrayLen(len(values)); err != nil {
			return err
		}
		for _, value := range values {
			if err := encodeOpaqueMsgpack(encoder, value); err != nil {
				return err
			}
		}
		return nil
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return err
	}
	if number, ok := value.(json.Number); ok {
		return encodeOpaqueNumber(encoder, number)
	}
	return encoder.Encode(value)
}

// MsgpackJSON converts opaque MessagePack values to ordered, lossless JSON.
func MsgpackJSON(data []byte) (json.RawMessage, error) {
	if err := CheckMsgpack(data); err != nil {
		return nil, err
	}
	code := data[0]
	switch {
	case msgpcode.IsFixedMap(code) || code == msgpcode.Map16 || code == msgpcode.Map32:
		fields, err := MsgpackFields(data)
		if err != nil {
			return nil, err
		}
		for i := range fields {
			raw, ok := fields[i].Value.(msgpack.RawMessage)
			if !ok {
				return nil, errors.New("expected raw MessagePack field")
			}
			value, err := MsgpackJSON(raw)
			if err != nil {
				return nil, At(fields[i].Name, err)
			}
			fields[i].Value = value
		}
		return EncodeObject(fields)
	case msgpcode.IsFixedArray(code) || code == msgpcode.Array16 || code == msgpcode.Array32:
		values, err := MsgpackList(data, MsgpackJSON)
		if err != nil {
			return nil, err
		}
		return EncodeJSON(values)
	case code == msgpcode.Nil || code == msgpcode.True || code == msgpcode.False ||
		msgpcode.IsString(code) || msgpcode.IsFixedNum(code) ||
		code >= msgpcode.Uint8 && code <= msgpcode.Int64 || code == msgpcode.Float || code == msgpcode.Double:
		value, err := msgpack.NewDecoder(bytes.NewReader(data)).DecodeInterface()
		if err != nil {
			return nil, err
		}
		// A float stays a float, including an integral float and negative zero.
		var number float64
		floating := false
		switch v := value.(type) {
		case float32:
			number, floating = float64(v), true
		case float64:
			number, floating = v, true
		}
		if floating {
			if math.IsNaN(number) || math.IsInf(number, 0) {
				return []byte("null"), nil
			}
			return EncodeJSON(number)
		}
		return EncodeJSON(value)
	default:
		return nil, errors.New("MessagePack value is not JSON")
	}
}

// encodeOpaqueNumber encodes a number with no fraction or exponent, other than -0,
// as an integer when it fits 64 bits, and every other number as a float64.
func encodeOpaqueNumber(encoder *msgpack.Encoder, number json.Number) error {
	text := string(number)
	if !strings.ContainsAny(text, ".eE") && text != "-0" {
		if strings.HasPrefix(text, "-") {
			if n, err := strconv.ParseInt(text, 10, 64); err == nil {
				return encoder.EncodeInt(n)
			}
		} else if n, err := strconv.ParseUint(text, 10, 64); err == nil {
			return encoder.EncodeUint(n)
		}
	}
	n, err := strconv.ParseFloat(text, 64)
	if err != nil {
		return err
	}
	return encoder.EncodeFloat64(n)
}

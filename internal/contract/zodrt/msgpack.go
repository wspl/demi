package zodrt

import (
	"bytes"
	"fmt"
	"maps"
	"math"
	"math/big"
	"slices"
	"time"
	"unicode/utf8"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/vmihailenco/msgpack/v5/msgpcode"
)

// DecodeMsgpack decodes one MessagePack value into a tree, as `decode` of
// @msgpack/msgpack does: every number becomes a float64, bin becomes []byte,
// the timestamp extension becomes a time with a JavaScript Date's
// millisecond precision, a map becomes an Object in wire order, and bytes
// after the value reject the frame. It also rejects what TypeScript never
// writes: nesting deeper than MaxDepth, strings that are not UTF-8, and map
// keys that are not strings (@msgpack/msgpack also reads number keys, but
// no TypeScript end writes them).
func DecodeMsgpack(data []byte) (any, error) {
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	value, err := decodeMsgpackValue(decoder, reader, 0)
	if err != nil {
		return nil, fmt.Errorf("malformed MessagePack: %w", err)
	}
	if reader.Len() > 0 {
		return nil, fmt.Errorf("malformed MessagePack: %d extra bytes after the value", reader.Len())
	}
	return value, nil
}

// decodeMsgpackValue reads arrays, maps and strings itself so that it can
// bound the nesting before it recurses and check strings; the library's
// generic decoding recurses without a bound. Scalars are the library's.
func decodeMsgpackValue(decoder *msgpack.Decoder, reader *bytes.Reader, depth int) (any, error) {
	code, err := decoder.PeekCode()
	if err != nil {
		return nil, err
	}
	switch {
	case msgpcode.IsFixedArray(code) || code == msgpcode.Array16 || code == msgpcode.Array32:
		if depth >= MaxDepth {
			return nil, fmt.Errorf("nesting deeper than %d levels", MaxDepth)
		}
		length, err := decoder.DecodeArrayLen()
		if err != nil {
			return nil, err
		}
		// Each item takes at least a byte, so a length the frame cannot
		// hold allocates no more than the frame.
		items := make([]any, 0, min(length, reader.Len()))
		for range length {
			item, err := decodeMsgpackValue(decoder, reader, depth+1)
			if err != nil {
				return nil, err
			}
			items = append(items, item)
		}
		return items, nil
	case msgpcode.IsFixedMap(code) || code == msgpcode.Map16 || code == msgpcode.Map32:
		if depth >= MaxDepth {
			return nil, fmt.Errorf("nesting deeper than %d levels", MaxDepth)
		}
		length, err := decoder.DecodeMapLen()
		if err != nil {
			return nil, err
		}
		fields := make(Object, 0, min(length, reader.Len()))
		positions := make(map[string]int, min(length, reader.Len()))
		for range length {
			keyCode, err := decoder.PeekCode()
			if err != nil {
				return nil, err
			}
			if !msgpcode.IsString(keyCode) {
				return nil, fmt.Errorf("a map key is not a string")
			}
			key, err := decodeMsgpackString(decoder)
			if err != nil {
				return nil, err
			}
			value, err := decodeMsgpackValue(decoder, reader, depth+1)
			if err != nil {
				return nil, err
			}
			fields = setField(fields, positions, key, value)
		}
		return fields, nil
	case msgpcode.IsString(code):
		return decodeMsgpackString(decoder)
	}
	value, err := decoder.DecodeInterface()
	if err != nil {
		return nil, err
	}
	return normalizeMsgpackScalar(value)
}

func decodeMsgpackString(decoder *msgpack.Decoder) (string, error) {
	text, err := decoder.DecodeString()
	if err != nil {
		return "", err
	}
	if !utf8.ValidString(text) {
		return "", fmt.Errorf("a string is not UTF-8")
	}
	return text, nil
}

// normalizeMsgpackScalar turns the typed numbers and times of the library's
// generic decoding into the JavaScript values @msgpack/msgpack produces.
func normalizeMsgpackScalar(value any) (any, error) {
	switch node := value.(type) {
	case int8:
		return float64(node), nil
	case int16:
		return float64(node), nil
	case int32:
		return float64(node), nil
	case int64:
		return float64(node), nil
	case uint8:
		return float64(node), nil
	case uint16:
		return float64(node), nil
	case uint32:
		return float64(node), nil
	case uint64:
		return float64(node), nil
	case float32:
		return float64(node), nil
	case time.Time:
		// `new Date(sec * 1000 + nsec / 1e6)`: the Date constructor truncates
		// the milliseconds toward zero.
		millis := math.Trunc(float64(node.Unix())*1e3 + float64(node.Nanosecond())/1e6)
		return time.UnixMilli(int64(millis)).UTC(), nil
	case nil, bool, float64, []byte:
		return node, nil
	}
	return nil, fmt.Errorf("unsupported %T", value)
}

// EncodeMsgpack encodes a tree as `encode` of @msgpack/msgpack does: objects
// in field order, a number as the smallest integer when it is a safe integer
// and as a float64 otherwise, bytes as bin, a time as the timestamp
// extension. A value of another Go type is encoded by the library.
func EncodeMsgpack(value any) ([]byte, error) {
	var out bytes.Buffer
	encoder := msgpack.NewEncoder(&out)
	encoder.UseCompactInts(true)
	encoder.SetSortMapKeys(true)
	if err := writeMsgpack(encoder, value); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

// writeMsgpack walks the tree itself because the library's generic encoding
// has no ordered object and compacts integral floats beyond JavaScript's safe
// integers, which @msgpack/msgpack writes as float64.
func writeMsgpack(encoder *msgpack.Encoder, value any) error {
	switch node := value.(type) {
	case nil:
		return encoder.EncodeNil()
	case bool:
		return encoder.EncodeBool(node)
	case float64:
		if math.Trunc(node) == node && math.Abs(node) <= maxSafeInteger {
			return encoder.EncodeInt(int64(node))
		}
		return encoder.EncodeFloat64(node)
	case int64:
		return encoder.EncodeInt(node)
	case string:
		return encoder.EncodeString(node)
	case []byte:
		// The library writes a nil slice as nil; an empty Uint8Array is bin.
		if node == nil {
			node = []byte{}
		}
		return encoder.EncodeBytes(node)
	case time.Time:
		// A JavaScript Date holds milliseconds; TypeScript and the JSON
		// codec write no finer time.
		return encoder.EncodeTime(node.Truncate(time.Millisecond))
	case *big.Int:
		return fmt.Errorf("MessagePack cannot carry a big integer")
	case []any:
		if err := encoder.EncodeArrayLen(len(node)); err != nil {
			return err
		}
		for _, item := range node {
			if err := writeMsgpack(encoder, item); err != nil {
				return err
			}
		}
		return nil
	case Object:
		if err := encoder.EncodeMapLen(len(node)); err != nil {
			return err
		}
		for _, field := range jsPropertyOrder(node) {
			if err := encoder.EncodeString(field.Key); err != nil {
				return err
			}
			if err := writeMsgpack(encoder, field.Value); err != nil {
				return err
			}
		}
		return nil
	case map[string]any:
		// A Go map has no order; its keys are written sorted.
		if err := encoder.EncodeMapLen(len(node)); err != nil {
			return err
		}
		for _, key := range slices.Sorted(maps.Keys(node)) {
			if err := encoder.EncodeString(key); err != nil {
				return err
			}
			if err := writeMsgpack(encoder, node[key]); err != nil {
				return err
			}
		}
		return nil
	}
	return encoder.Encode(value)
}

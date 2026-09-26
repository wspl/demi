package zodrt

import (
	"bytes"
	"fmt"
	"maps"
	"math"
	"math/big"
	"slices"
	"time"

	"github.com/vmihailenco/msgpack/v5"
)

// DecodeMsgpack decodes one MessagePack value into a tree, as `decode` of
// @msgpack/msgpack does: every number becomes a float64, bin becomes []byte,
// the timestamp extension becomes a time with a JavaScript Date's
// millisecond precision, and bytes after the value reject the frame.
func DecodeMsgpack(data []byte) (any, error) {
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	value, err := decoder.DecodeInterface()
	if err != nil {
		return nil, fmt.Errorf("malformed MessagePack: %w", err)
	}
	if reader.Len() > 0 {
		return nil, fmt.Errorf("malformed MessagePack: %d extra bytes after the value", reader.Len())
	}
	return normalizeMsgpack(value)
}

// normalizeMsgpack turns the typed numbers and times of the library's generic
// decoding into the JavaScript values @msgpack/msgpack produces.
func normalizeMsgpack(value any) (any, error) {
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
	case []any:
		for index, item := range node {
			normalized, err := normalizeMsgpack(item)
			if err != nil {
				return nil, err
			}
			node[index] = normalized
		}
		return node, nil
	case map[string]any:
		for key, item := range node {
			normalized, err := normalizeMsgpack(item)
			if err != nil {
				return nil, err
			}
			node[key] = normalized
		}
		return node, nil
	case nil, bool, float64, string, []byte:
		return node, nil
	}
	return nil, fmt.Errorf("malformed MessagePack: unsupported %T", value)
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
		return encoder.EncodeTime(node)
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
		for _, field := range node {
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

package wire

import (
	"math"
	"time"
	"unicode/utf8"

	"github.com/tinylib/msgp/msgp"
)

// MPRead reads one primitive and hides library diagnostics, which may contain values.
func MPRead[T any](data []byte, read func([]byte) (T, []byte, error)) (T, error) {
	value, rest, err := read(data)
	if err != nil {
		return value, &InvalidError{Rule: "invalid MessagePack value"}
	}
	if len(rest) != 0 {
		return value, &InvalidError{Rule: "trailing MessagePack data"}
	}
	return value, nil
}

// MPString requires a UTF-8 string; binary is a different wire kind.
func MPString(data []byte) (string, error) {
	if msgp.NextType(data) != msgp.StrType {
		return "", &InvalidError{Rule: "expected string"}
	}
	value, err := MPRead(data, msgp.ReadStringBytes)
	if err == nil && !utf8.ValidString(value) {
		err = &InvalidError{Rule: "invalid UTF-8"}
	}
	return value, err
}

// MPBytes requires bin, never an array or a string, and owns the result.
func MPBytes(data []byte) ([]byte, error) {
	if msgp.NextType(data) != msgp.BinType {
		return nil, &InvalidError{Rule: "expected binary bytes"}
	}
	return MPRead(data, func(b []byte) ([]byte, []byte, error) { return msgp.ReadBytesBytes(b, nil) })
}

// MPTimestamp reads the protocol's milliseconds from the standard timestamp extension.
func MPTimestamp(data []byte) (int64, error) {
	t, err := MPRead(data, msgp.ReadTimeBytes)
	if err != nil {
		return 0, err
	}
	// The library accepts its private time extension too. Require the standard
	// type through its extension reader, which also accepts nonminimal headers.
	extension := msgp.RawExtension{Type: -1}
	if _, err := msgp.ReadExtensionBytes(data, &extension); err != nil {
		return 0, &InvalidError{Rule: "expected timestamp"}
	}
	seconds := t.Unix()
	millis := int64(t.Nanosecond() / 1000000)
	if seconds > math.MaxInt64/1000 || seconds < math.MinInt64/1000 || seconds*1000 > math.MaxInt64-millis {
		return 0, &InvalidError{Rule: "timestamp out of range"}
	}
	return seconds*1000 + millis, nil
}

// MPAppendTimestamp uses the shortest standard extension, as the Rust wire does.
func MPAppendTimestamp(data []byte, millis int64) []byte {
	// msgp v1.4.0 chooses the eight-byte form at epoch zero; Rust and
	// the protocol require the shortest form, including zero.
	if millis == 0 {
		return append(data, 0xd6, 0xff, 0, 0, 0, 0)
	}
	return msgp.AppendTimeExt(data, time.UnixMilli(millis))
}

// MPMember retains map order for contracts whose discriminator precedes its payload.
type MPMember struct {
	Name string
	Data []byte
}

// MPObject splits a wire map, refusing duplicate and non-string member names.
// Allocation follows actual input, never an untrusted declared container size.
func MPObject(data []byte) ([]MPMember, error) {
	count, rest, err := msgp.ReadMapHeaderBytes(data)
	if err != nil {
		return nil, &InvalidError{Rule: "expected map"}
	}
	var fields []MPMember
	seen := map[string]bool{}
	for range count {
		name, next, err := msgp.ReadStringBytes(rest)
		if msgp.NextType(rest) != msgp.StrType || err != nil || !utf8.ValidString(name) {
			return nil, &InvalidError{Rule: "expected member name"}
		}
		if seen[name] {
			return nil, &InvalidError{Path: name, Rule: "duplicate member"}
		}
		seen[name] = true
		tail, err := msgp.Skip(next)
		if err != nil {
			return nil, &InvalidError{Path: name, Rule: "invalid MessagePack value"}
		}
		fields = append(fields, MPMember{name, next[:len(next)-len(tail)]})
		rest = tail
	}
	if len(rest) != 0 {
		return nil, &InvalidError{Rule: "trailing MessagePack data"}
	}
	return fields, nil
}

// MPArray decodes each element at its indexed wire path.
func MPArray[T any](data []byte, decode func([]byte) (T, error)) ([]T, error) {
	count, rest, err := msgp.ReadArrayHeaderBytes(data)
	if err != nil {
		return nil, &InvalidError{Rule: "expected array"}
	}
	values := []T{}
	for range count {
		tail, err := msgp.Skip(rest)
		if err != nil {
			return nil, &InvalidError{Path: Index(len(values)), Rule: "invalid MessagePack value"}
		}
		value, err := decode(rest[:len(rest)-len(tail)])
		if err != nil {
			return nil, In(Index(len(values)), err)
		}
		values = append(values, value)
		rest = tail
	}
	if len(rest) != 0 {
		return nil, &InvalidError{Rule: "trailing MessagePack data"}
	}
	return values, nil
}

// MPMap decodes the values of a string-keyed wire map with their paths.
func MPMap[K ~string, V any](data []byte, decode func([]byte) (V, error)) (map[K]V, error) {
	fields, err := MPObject(data)
	if err != nil {
		return nil, err
	}
	result := map[K]V{}
	for _, field := range fields {
		value, err := decode(field.Data)
		if err != nil {
			return nil, In(Key(field.Name), err)
		}
		result[K(field.Name)] = value
	}
	return result, nil
}

// MPNullable decodes a nullable value, preserving nil.
func MPNullable[T any](data []byte, decode func([]byte) (T, error)) (*T, error) {
	if len(data) == 1 && msgp.IsNil(data) {
		return nil, nil
	}
	value, err := decode(data)
	if err != nil {
		return nil, err
	}
	return &value, nil
}

// MPAdjacent extracts a tagged union's payload, enforcing its closed envelope.
func MPAdjacent(data []byte, tagName, contentName, expected string) ([]byte, error) {
	fields, err := MPObject(data)
	if err != nil {
		return nil, err
	}
	var content []byte
	found := false
	for _, field := range fields {
		switch field.Name {
		case tagName:
			tag, err := MPString(field.Data)
			if err != nil {
				return nil, In(tagName, err)
			}
			if tag != expected {
				return nil, UnknownTag(tagName, expected)
			}
			found = true
		case contentName:
			content = field.Data
		default:
			return nil, Unknown(field.Name)
		}
	}
	if !found {
		return nil, Required(tagName)
	}
	if content == nil {
		return nil, Required(contentName)
	}
	return content, nil
}

// MPFloat accepts the numeric wire kinds and refuses values JSON cannot carry.
func MPFloat(data []byte) (float64, error) {
	var value float64
	var err error
	// msgp's float reader accepts only float prefixes. Read integer primitives
	// separately for the integer-to-float coercion that serde permits.
	switch msgp.NextType(data) {
	case msgp.IntType:
		var n int64
		n, err = MPRead(data, msgp.ReadInt64Bytes)
		value = float64(n)
	case msgp.UintType:
		var n uint64
		n, err = MPRead(data, msgp.ReadUint64Bytes)
		value = float64(n)
	default:
		value, err = MPRead(data, msgp.ReadFloat64Bytes)
	}
	if err != nil {
		return 0, err
	}
	if math.IsNaN(value) || math.IsInf(value, 0) {
		return 0, &InvalidError{Rule: "expected finite number"}
	}
	return value, nil
}

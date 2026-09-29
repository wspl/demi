package wire

import (
	"bytes"
)

// DecodeNullable decodes data, one JSON document that is a value or a JSON null,
// as a T with decode, or as nil when it is a null: serde's Option.
func DecodeNullable[T any](data []byte, decode func([]byte) (T, error)) (*T, error) {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return nil, nil
	}
	value, err := decode(data)
	if err != nil {
		return nil, err
	}
	return &value, nil
}

// EncodeNullable returns the JSON of value with encode, or a JSON null for nil.
func EncodeNullable[T any](value *T, encode func(T) ([]byte, error)) ([]byte, error) {
	if value == nil {
		return []byte("null"), nil
	}
	return encode(*value)
}

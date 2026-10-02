package contract

import (
	"bytes"
	"reflect"
	"strconv"
	"strings"

	"github.com/vmihailenco/msgpack/v5/msgpcode"
)

type integer interface {
	~int | ~int8 | ~int16 | ~int32 | ~int64 | ~uint | ~uint8 | ~uint16 | ~uint32 | ~uint64
}

// Integer decodes a contract integer accepting Rust FromStr decimal strings.
func Integer[T integer](data []byte) (T, error) {
	data = bytes.TrimLeft(data, " \t\r\n")
	if len(data) == 0 || data[0] != '"' {
		return Decode[T](data)
	}
	text, err := Decode[string](data)
	if err != nil {
		return 0, err
	}
	return parseInteger[T](text)
}

// MsgpackInteger applies the same numbered-input rule to MessagePack strings.
func MsgpackInteger[T integer](data []byte) (T, error) {
	if len(data) == 0 || !msgpcode.IsString(data[0]) {
		return DecodeMsgpack[T](data)
	}
	text, err := DecodeMsgpack[string](data)
	if err != nil {
		return 0, err
	}
	return parseInteger[T](text)
}

// parseInteger uses the contract's integer width and Rust's decimal sign rules.
func parseInteger[T integer](text string) (T, error) {
	var value T
	target := reflect.ValueOf(&value).Elem()
	if target.Kind() >= reflect.Uint && target.Kind() <= reflect.Uint64 {
		number, err := strconv.ParseUint(strings.TrimPrefix(text, "+"), 10, target.Type().Bits())
		if err != nil {
			return 0, err
		}
		target.SetUint(number)
	} else {
		number, err := strconv.ParseInt(text, 10, target.Type().Bits())
		if err != nil {
			return 0, err
		}
		target.SetInt(number)
	}
	return value, nil
}

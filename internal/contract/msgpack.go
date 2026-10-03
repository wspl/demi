package contract

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"reflect"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/vmihailenco/msgpack/v5/msgpcode"
)

// MsgpackNull reports an explicit MessagePack nil.
func MsgpackNull(data []byte) bool { return len(data) == 1 && data[0] == msgpcode.Nil }

// MsgpackObject preserves presence and refuses duplicate names before dispatch.
func MsgpackObject(data []byte) (map[string][]byte, error) {
	fields, err := MsgpackFields(data)
	if err != nil {
		return nil, err
	}
	output := make(map[string][]byte, len(fields))
	for _, field := range fields {
		raw, ok := field.Value.(msgpack.RawMessage)
		if !ok {
			return nil, At(field.Name, errors.New("expected raw MessagePack field"))
		}
		output[field.Name] = raw
	}
	return output, nil
}

// DecodeMsgpack checks scalar kinds and integer narrowing before library decoding.
// Floating-point targets also accept integer tokens.
// Generated object methods own schema-dependent checks.
func DecodeMsgpack[T any](data []byte) (T, error) {
	var value T
	if len(data) == 0 || MsgpackNull(data) {
		return value, errors.New("null or empty MessagePack")
	}
	target := reflect.TypeFor[T]()
	if _, ok := any(&value).(msgpack.Unmarshaler); !ok {
		if target.Kind() == reflect.Float32 || target.Kind() == reflect.Float64 {
			err := decodeMsgpackFloat(data, target, reflect.ValueOf(&value).Elem())
			return value, err
		}
		if err := checkMsgpackScalar(data, target, reflect.ValueOf(&value).Elem()); err != nil {
			return value, err
		}
	}
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	if err := decoder.Decode(&value); err != nil {
		return value, err
	}
	if reader.Len() != 0 {
		return value, errors.New("trailing MessagePack data")
	}
	if target.Kind() == reflect.String && !utf8.ValidString(reflect.ValueOf(value).String()) {
		return value, errors.New("invalid UTF-8")
	}
	return value, nil
}

// MsgpackList decodes arrays without accepting nil or binary as arrays.
func MsgpackList[T any](data []byte, decode func([]byte) (T, error)) ([]T, error) {
	if err := CheckMsgpack(data); err != nil {
		return nil, err
	}
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	code, err := decoder.PeekCode()
	if err != nil {
		return nil, err
	}
	if !msgpcode.IsFixedArray(code) && code != msgpcode.Array16 && code != msgpcode.Array32 {
		return nil, errors.New("expected array")
	}
	n, err := decoder.DecodeArrayLen()
	if err != nil {
		return nil, err
	}
	if n > len(data) {
		return nil, errors.New("invalid array length")
	}
	output := make([]T, n)
	for i := range output {
		raw, err := decoder.DecodeRaw()
		if err != nil {
			return nil, err
		}
		output[i], err = decode(raw)
		if err != nil {
			return nil, At(fmt.Sprintf("[%d]", i), err)
		}
	}
	if reader.Len() != 0 {
		return nil, errors.New("trailing MessagePack data")
	}
	return output, nil
}

// MsgpackRecord checks every string key and nullable record value.
func MsgpackRecord[T any](data []byte, decode func([]byte) (T, error), nullable bool) (map[string]T, error) {
	return MsgpackKeyedRecord[string](data, decode, nullable)
}

// MsgpackKeyedRecord retains named string keys for generated key validation.
func MsgpackKeyedRecord[K ~string, T any](data []byte, decode func([]byte) (T, error), nullable bool) (map[K]T, error) {
	raw, err := MsgpackObject(data)
	if err != nil {
		return nil, err
	}
	output := make(map[K]T, len(raw))
	for key, item := range raw {
		var value T
		if !nullable || !MsgpackNull(item) {
			value, err = decode(item)
			if err != nil {
				return nil, At(key, err)
			}
		}
		output[K(key)] = value
	}
	return output, nil
}

// EncodeMsgpack uses the wire's compact integer representation.
func EncodeMsgpack(value any) ([]byte, error) {
	var output bytes.Buffer
	encoder := msgpack.NewEncoder(&output)
	encoder.UseCompactInts(true)
	if err := encodeMsgpackValue(encoder, reflect.ValueOf(value)); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}

// EncodeMsgpackObject preserves declaration order, including the union tag.
func EncodeMsgpackObject(fields []Field) ([]byte, error) {
	var output bytes.Buffer
	encoder := msgpack.NewEncoder(&output)
	encoder.UseCompactInts(true)
	if err := encoder.EncodeMapLen(len(fields)); err != nil {
		return nil, err
	}
	for _, field := range fields {
		if err := encoder.EncodeString(field.Name); err != nil {
			return nil, err
		}
		if err := encodeMsgpackValue(encoder, reflect.ValueOf(field.Value)); err != nil {
			return nil, At(field.Name, err)
		}
	}
	return output.Bytes(), nil
}

// MsgpackTimestamp admits only the standard timestamp extension and milliseconds.
func MsgpackTimestamp(data []byte) (string, error) {
	seconds, nanos, err := msgpackTime(data)
	if err != nil {
		return "", err
	}
	if nanos%1e6 != 0 {
		return "", errors.New("timestamp is not whole milliseconds")
	}
	value := time.Unix(seconds, int64(nanos)).UTC().Format("2006-01-02T15:04:05.000Z")
	if err := Timestamp(value); err != nil {
		return "", err
	}
	return value, nil
}

// EncodeMsgpackTimestamp writes the shortest standard timestamp extension.
func EncodeMsgpackTimestamp(value string) ([]byte, error) {
	if err := Timestamp(value); err != nil {
		return nil, err
	}
	timestamp, err := time.Parse("2006-01-02T15:04:05.000Z", value)
	if err != nil {
		return nil, err
	}
	return EncodeMsgpack(timestamp)
}

// CheckMsgpack rejects excessive nesting and duplicate keys, including values
// inside fields a tolerant generated object will ignore.
func CheckMsgpack(data []byte) error {
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	if err := scanMsgpack(decoder, reader, 0); err != nil {
		return err
	}
	if reader.Len() != 0 {
		return errors.New("trailing MessagePack data")
	}
	return nil
}

func scanMsgpack(decoder *msgpack.Decoder, reader *bytes.Reader, depth int) error {
	if depth > 1000 {
		return errors.New("MessagePack nesting exceeds 1000")
	}
	code, err := decoder.PeekCode()
	if err != nil {
		return err
	}
	if msgpcode.IsFixedMap(code) || code == msgpcode.Map16 || code == msgpcode.Map32 {
		return scanMsgpackObject(decoder, reader, depth)
	}
	if msgpcode.IsFixedArray(code) || code == msgpcode.Array16 || code == msgpcode.Array32 {
		return scanMsgpackArray(decoder, reader, depth)
	}
	if msgpcode.IsString(code) {
		value, err := decoder.DecodeString()
		if err != nil {
			return err
		}
		if !utf8.ValidString(value) {
			return errors.New("invalid UTF-8")
		}
		return nil
	}
	_, err = decoder.DecodeRaw()
	return err
}

// MsgpackTimestampValue adapts a canonical timestamp field to the wire extension.
// It is an encoding adapter, not a domain timestamp type.
type MsgpackTimestampValue string

// MarshalMsgpack encodes the field as a timestamp extension.
func (v MsgpackTimestampValue) MarshalMsgpack() ([]byte, error) {
	return EncodeMsgpackTimestamp(string(v))
}

// encodeMsgpackValue fills the library's generic-map sorting gap. The library
// sorts only selected concrete map types; generated records can hold any type.
func encodeMsgpackValue(encoder *msgpack.Encoder, value reflect.Value) error {
	if !value.IsValid() {
		return encoder.EncodeNil()
	}
	switch value.Kind() {
	case reflect.Pointer, reflect.Interface, reflect.Map, reflect.Slice:
		if value.IsNil() {
			return encoder.EncodeNil()
		}
	}
	if raw, ok := value.Interface().(json.RawMessage); ok {
		return encodeOpaqueMsgpack(encoder, raw)
	}
	if _, ok := value.Interface().(msgpack.Marshaler); ok {
		return encoder.Encode(value.Interface())
	}
	switch value.Kind() {
	case reflect.Pointer, reflect.Interface:
		return encodeMsgpackValue(encoder, value.Elem())
	case reflect.Map:
		return encodeMsgpackRecord(encoder, value)
	case reflect.Slice:
		if value.Type().Elem().Kind() == reflect.Uint8 {
			return encoder.Encode(value.Interface())
		}
		if err := encoder.EncodeArrayLen(value.Len()); err != nil {
			return err
		}
		for i := 0; i < value.Len(); i++ {
			if err := encodeMsgpackValue(encoder, value.Index(i)); err != nil {
				return At(fmt.Sprintf("[%d]", i), err)
			}
		}
		return nil
	default:
		return encoder.Encode(value.Interface())
	}
}

// MsgpackTuple checks an external union tag and its exact positional arity.
func MsgpackTuple(data []byte, tag string, count int) ([][]byte, error) {
	object, err := MsgpackObject(data)
	if err != nil {
		return nil, err
	}
	raw, ok := object[tag]
	if len(object) != 1 || !ok {
		return nil, errors.New("invalid external union tag")
	}
	if count == 1 {
		return [][]byte{raw}, nil
	}
	fields, err := MsgpackList(raw, func(b []byte) ([]byte, error) { return b, nil })
	if err != nil {
		return nil, At(tag, err)
	}
	if len(fields) != count {
		return nil, At(tag, errors.New("invalid tuple length"))
	}
	return fields, nil
}

// EncodeMsgpackTuple writes one external tag, with a scalar for a single field.
func EncodeMsgpackTuple(tag string, fields []any) ([]byte, error) {
	var value any = fields
	if len(fields) == 1 {
		value = fields[0]
	}
	return EncodeMsgpackObject([]Field{{Name: tag, Value: value}})
}

// MsgpackMillis reads the runner timestamp, truncating sub-millisecond precision.
func MsgpackMillis(data []byte) (int64, error) {
	seconds, nanos, err := msgpackTime(data)
	if err != nil {
		return 0, err
	}
	if seconds > math.MaxInt64/1000 || seconds < math.MinInt64/1000 {
		return 0, errors.New("timestamp overflow")
	}
	millis := seconds * 1000
	fraction := int64(nanos / 1e6)
	if millis > math.MaxInt64-fraction {
		return 0, errors.New("timestamp overflow")
	}
	return millis + fraction, nil
}

// EncodeMsgpackMillis writes signed epoch milliseconds in the shortest form.
func EncodeMsgpackMillis(value int64) ([]byte, error) {
	return EncodeMsgpack(time.UnixMilli(value))
}

// msgpackTime validates extension bytes before any time library normalizes them.
func msgpackTime(data []byte) (int64, uint32, error) {
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	id, n, err := decoder.DecodeExtHeader()
	if err != nil {
		return 0, 0, err
	}
	if id != -1 || n != 4 && n != 8 && n != 12 || reader.Len() != n {
		return 0, 0, errors.New("invalid timestamp extension")
	}
	payload := data[len(data)-n:]
	var seconds int64
	var nanos uint32
	switch n {
	case 4:
		seconds = int64(binary.BigEndian.Uint32(payload))
	case 8:
		value := binary.BigEndian.Uint64(payload)
		seconds = int64(value & 0x3ffffffff)
		nanos = uint32(value >> 34)
	case 12:
		nanos = binary.BigEndian.Uint32(payload)
		seconds = int64(binary.BigEndian.Uint64(payload[4:]))
	}
	if nanos >= 1e9 {
		return 0, 0, errors.New("invalid timestamp nanoseconds")
	}
	return seconds, nanos, nil
}

func checkMsgpackScalar(data []byte, target reflect.Type, destination reflect.Value) error {
	code := data[0]
	switch target.Kind() {
	case reflect.String:
		if !msgpcode.IsString(code) {
			return errors.New("expected string")
		}
	case reflect.Bool:
		if code != msgpcode.True && code != msgpcode.False {
			return errors.New("expected boolean")
		}
	case reflect.Int,
		reflect.Int8,
		reflect.Int16,
		reflect.Int32,
		reflect.Int64,
		reflect.Uint,
		reflect.Uint8,
		reflect.Uint16,
		reflect.Uint32,
		reflect.Uint64:
		if !msgpcode.IsFixedNum(code) && (code < msgpcode.Uint8 || code > msgpcode.Int64) {
			return errors.New("expected integer")
		}
		return checkMsgpackInteger(data, destination)
	case reflect.Slice:
		if target.Elem().Kind() != reflect.Uint8 ||
			code != msgpcode.Bin8 && code != msgpcode.Bin16 && code != msgpcode.Bin32 {
			return errors.New("expected binary")
		}
	default:
		return fmt.Errorf("no generated MessagePack decoder for %s", target)
	}
	return nil
}

func checkMsgpackInteger(data []byte, destination reflect.Value) error {
	value, err := msgpack.NewDecoder(bytes.NewReader(data)).DecodeInterface()
	if err != nil {
		return err
	}
	number := reflect.ValueOf(value)
	if destination.Kind() >= reflect.Uint && destination.Kind() <= reflect.Uint64 {
		return checkMsgpackUnsigned(number, destination)
	}
	return checkMsgpackSigned(number, destination)
}

func checkMsgpackUnsigned(number, destination reflect.Value) error {
	var u uint64
	if number.Kind() >= reflect.Int && number.Kind() <= reflect.Int64 {
		if number.Int() < 0 {
			return errors.New("negative unsigned integer")
		}
		u = uint64(number.Int())
	} else {
		u = number.Uint()
	}
	if destination.OverflowUint(u) {
		return errors.New("integer overflow")
	}
	return nil
}

func checkMsgpackSigned(number, destination reflect.Value) error {
	var i int64
	if number.Kind() >= reflect.Uint && number.Kind() <= reflect.Uint64 {
		if number.Uint() > math.MaxInt64 {
			return errors.New("integer overflow")
		}
		i = int64(number.Uint())
	} else {
		i = number.Int()
	}
	if destination.OverflowInt(i) {
		return errors.New("integer overflow")
	}
	return nil
}

func decodeMsgpackFloat(data []byte, target reflect.Type, destination reflect.Value) error {
	code := data[0]
	if code != msgpcode.Float && code != msgpcode.Double && !msgpcode.IsFixedNum(code) &&
		(code < msgpcode.Uint8 || code > msgpcode.Int64) {
		return errors.New("expected number")
	}
	// The library's float decoder routes integers through int64 (which
	// wraps large uint64 values), and float32 refuses float64 tokens.
	// Decode the token in its own representation before converting it.
	reader := bytes.NewReader(data)
	n, err := msgpack.NewDecoder(reader).DecodeInterface()
	if err != nil {
		return err
	}
	if reader.Len() != 0 {
		return errors.New("trailing MessagePack data")
	}
	destination.Set(reflect.ValueOf(n).Convert(target))
	return nil
}

func scanMsgpackObject(decoder *msgpack.Decoder, reader *bytes.Reader, depth int) error {
	n, err := decoder.DecodeMapLen()
	if err != nil {
		return err
	}
	if n > reader.Len()/2 {
		return errors.New("invalid object length")
	}
	seen := map[string]bool{}
	for i := 0; i < n; i++ {
		code, err := decoder.PeekCode()
		if err != nil {
			return err
		}
		if !msgpcode.IsString(code) {
			return errors.New("expected string key")
		}
		key, err := decoder.DecodeString()
		if err != nil {
			return err
		}
		if !utf8.ValidString(key) {
			return errors.New("invalid UTF-8")
		}
		if seen[key] {
			return At(key, errors.New("duplicate key"))
		}
		seen[key] = true
		if err := scanMsgpack(decoder, reader, depth+1); err != nil {
			return At(key, err)
		}
	}
	return nil
}

func scanMsgpackArray(decoder *msgpack.Decoder, reader *bytes.Reader, depth int) error {
	n, err := decoder.DecodeArrayLen()
	if err != nil {
		return err
	}
	if n > reader.Len() {
		return errors.New("invalid array length")
	}
	for i := 0; i < n; i++ {
		if err := scanMsgpack(decoder, reader, depth+1); err != nil {
			return At(fmt.Sprintf("[%d]", i), err)
		}
	}
	return nil
}

// encodeMsgpackRecord sorts every string-keyed record, including named key types.
func encodeMsgpackRecord(encoder *msgpack.Encoder, value reflect.Value) error {
	if value.Type().Key().Kind() != reflect.String {
		return errors.New("record keys must be strings")
	}
	keys := value.MapKeys()
	slices.SortFunc(keys, func(a, b reflect.Value) int { return strings.Compare(a.String(), b.String()) })
	if err := encoder.EncodeMapLen(len(keys)); err != nil {
		return err
	}
	for _, key := range keys {
		if err := encoder.EncodeString(key.String()); err != nil {
			return err
		}
		if err := encodeMsgpackValue(encoder, value.MapIndex(key)); err != nil {
			return At(key.String(), err)
		}
	}
	return nil
}

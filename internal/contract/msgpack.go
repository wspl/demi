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
	out := make(map[string][]byte, len(fields))
	for _, field := range fields {
		raw, ok := field.Value.(msgpack.RawMessage)
		if !ok {
			return nil, At(field.Name, errors.New("expected raw MessagePack field"))
		}
		out[field.Name] = raw
	}
	return out, nil
}

// DecodeMsgpack refuses scalar coercion and narrowing before library decoding.
// Generated object methods own schema-dependent checks.
func DecodeMsgpack[T any](data []byte) (T, error) {
	var value T
	if len(data) == 0 || MsgpackNull(data) {
		return value, errors.New("null or empty MessagePack")
	}
	target := reflect.TypeFor[T]()
	if _, ok := any(&value).(msgpack.Unmarshaler); !ok {
		code := data[0]
		switch target.Kind() {
		case reflect.String:
			if !msgpcode.IsString(code) {
				return value, errors.New("expected string")
			}
		case reflect.Bool:
			if code != msgpcode.True && code != msgpcode.False {
				return value, errors.New("expected boolean")
			}
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64, reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
			if !msgpcode.IsFixedNum(code) && (code < msgpcode.Uint8 || code > msgpcode.Int64) {
				return value, errors.New("expected integer")
			}
			v, err := msgpack.NewDecoder(bytes.NewReader(data)).DecodeInterface()
			if err != nil {
				return value, err
			}
			n := reflect.ValueOf(v)
			dst := reflect.ValueOf(&value).Elem()
			if target.Kind() >= reflect.Uint && target.Kind() <= reflect.Uint64 {
				var u uint64
				if n.Kind() >= reflect.Int && n.Kind() <= reflect.Int64 {
					if n.Int() < 0 {
						return value, errors.New("negative unsigned integer")
					}
					u = uint64(n.Int())
				} else {
					u = n.Uint()
				}
				if dst.OverflowUint(u) {
					return value, errors.New("integer overflow")
				}
			} else {
				var i int64
				if n.Kind() >= reflect.Uint && n.Kind() <= reflect.Uint64 {
					if n.Uint() > math.MaxInt64 {
						return value, errors.New("integer overflow")
					}
					i = int64(n.Uint())
				} else {
					i = n.Int()
				}
				if dst.OverflowInt(i) {
					return value, errors.New("integer overflow")
				}
			}
		case reflect.Float32, reflect.Float64:
			if code != msgpcode.Float && code != msgpcode.Double {
				return value, errors.New("expected float")
			}
		case reflect.Slice:
			if target.Elem().Kind() != reflect.Uint8 || code != msgpcode.Bin8 && code != msgpcode.Bin16 && code != msgpcode.Bin32 {
				return value, errors.New("expected binary")
			}
		default:
			return value, fmt.Errorf("no generated MessagePack decoder for %s", target)
		}
	}
	r := bytes.NewReader(data)
	d := msgpack.NewDecoder(r)
	if err := d.Decode(&value); err != nil {
		return value, err
	}
	if r.Len() != 0 {
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
	r := bytes.NewReader(data)
	d := msgpack.NewDecoder(r)
	code, err := d.PeekCode()
	if err != nil {
		return nil, err
	}
	if !msgpcode.IsFixedArray(code) && code != msgpcode.Array16 && code != msgpcode.Array32 {
		return nil, errors.New("expected array")
	}
	n, err := d.DecodeArrayLen()
	if err != nil {
		return nil, err
	}
	if n > len(data) {
		return nil, errors.New("invalid array length")
	}
	out := make([]T, n)
	for i := range out {
		raw, err := d.DecodeRaw()
		if err != nil {
			return nil, err
		}
		out[i], err = decode(raw)
		if err != nil {
			return nil, At(fmt.Sprintf("[%d]", i), err)
		}
	}
	if r.Len() != 0 {
		return nil, errors.New("trailing MessagePack data")
	}
	return out, nil
}

// MsgpackRecord checks every string key and nullable record value.
func MsgpackRecord[T any](data []byte, decode func([]byte) (T, error), nullable bool) (map[string]T, error) {
	raw, err := MsgpackObject(data)
	if err != nil {
		return nil, err
	}
	out := make(map[string]T, len(raw))
	for key, item := range raw {
		var value T
		if !nullable || !MsgpackNull(item) {
			value, err = decode(item)
			if err != nil {
				return nil, At(key, err)
			}
		}
		out[key] = value
	}
	return out, nil
}

// EncodeMsgpack uses the wire's compact integer representation.
func EncodeMsgpack(value any) ([]byte, error) {
	var out bytes.Buffer
	e := msgpack.NewEncoder(&out)
	e.UseCompactInts(true)
	if err := encodeMsgpackValue(e, reflect.ValueOf(value)); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

// EncodeMsgpackObject preserves declaration order, including the union tag.
func EncodeMsgpackObject(fields []Field) ([]byte, error) {
	var out bytes.Buffer
	e := msgpack.NewEncoder(&out)
	e.UseCompactInts(true)
	if err := e.EncodeMapLen(len(fields)); err != nil {
		return nil, err
	}
	for _, f := range fields {
		if err := e.EncodeString(f.Name); err != nil {
			return nil, err
		}
		if err := encodeMsgpackValue(e, reflect.ValueOf(f.Value)); err != nil {
			return nil, At(f.Name, err)
		}
	}
	return out.Bytes(), nil
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
	tm, err := time.Parse("2006-01-02T15:04:05.000Z", value)
	if err != nil {
		return nil, err
	}
	return EncodeMsgpack(tm)
}

// CheckMsgpack rejects excessive nesting and duplicate keys, including values
// inside fields a tolerant generated object will ignore.
func CheckMsgpack(data []byte) error {
	r := bytes.NewReader(data)
	d := msgpack.NewDecoder(r)
	if err := scanMsgpack(d, r, 0); err != nil {
		return err
	}
	if r.Len() != 0 {
		return errors.New("trailing MessagePack data")
	}
	return nil
}
func scanMsgpack(d *msgpack.Decoder, r *bytes.Reader, depth int) error {
	if depth > 1000 {
		return errors.New("MessagePack nesting exceeds 1000")
	}
	code, err := d.PeekCode()
	if err != nil {
		return err
	}
	if msgpcode.IsFixedMap(code) || code == msgpcode.Map16 || code == msgpcode.Map32 {
		n, err := d.DecodeMapLen()
		if err != nil {
			return err
		}
		if n > r.Len()/2 {
			return errors.New("invalid object length")
		}
		seen := map[string]bool{}
		for i := 0; i < n; i++ {
			code, err := d.PeekCode()
			if err != nil {
				return err
			}
			if !msgpcode.IsString(code) {
				return errors.New("expected string key")
			}
			key, err := d.DecodeString()
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
			if err := scanMsgpack(d, r, depth+1); err != nil {
				return At(key, err)
			}
		}
		return nil
	}
	if msgpcode.IsFixedArray(code) || code == msgpcode.Array16 || code == msgpcode.Array32 {
		n, err := d.DecodeArrayLen()
		if err != nil {
			return err
		}
		if n > r.Len() {
			return errors.New("invalid array length")
		}
		for i := 0; i < n; i++ {
			if err := scanMsgpack(d, r, depth+1); err != nil {
				return At(fmt.Sprintf("[%d]", i), err)
			}
		}
		return nil
	}
	if msgpcode.IsString(code) {
		value, err := d.DecodeString()
		if err != nil {
			return err
		}
		if !utf8.ValidString(value) {
			return errors.New("invalid UTF-8")
		}
		return nil
	}
	_, err = d.DecodeRaw()
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
func encodeMsgpackValue(e *msgpack.Encoder, v reflect.Value) error {
	if !v.IsValid() {
		return e.EncodeNil()
	}
	switch v.Kind() {
	case reflect.Pointer, reflect.Interface, reflect.Map, reflect.Slice:
		if v.IsNil() {
			return e.EncodeNil()
		}
	}
	if raw, ok := v.Interface().(json.RawMessage); ok {
		return encodeOpaqueMsgpack(e, raw)
	}
	if _, ok := v.Interface().(msgpack.Marshaler); ok {
		return e.Encode(v.Interface())
	}
	switch v.Kind() {
	case reflect.Pointer, reflect.Interface:
		return encodeMsgpackValue(e, v.Elem())
	case reflect.Map:
		if v.Type().Key().Kind() != reflect.String {
			return errors.New("record keys must be strings")
		}
		keys := v.MapKeys()
		slices.SortFunc(keys, func(a, b reflect.Value) int { return strings.Compare(a.String(), b.String()) })
		if err := e.EncodeMapLen(len(keys)); err != nil {
			return err
		}
		for _, key := range keys {
			if err := e.EncodeString(key.String()); err != nil {
				return err
			}
			if err := encodeMsgpackValue(e, v.MapIndex(key)); err != nil {
				return At(key.String(), err)
			}
		}
		return nil
	case reflect.Slice:
		if v.Type().Elem().Kind() == reflect.Uint8 {
			return e.Encode(v.Interface())
		}
		if err := e.EncodeArrayLen(v.Len()); err != nil {
			return err
		}
		for i := 0; i < v.Len(); i++ {
			if err := encodeMsgpackValue(e, v.Index(i)); err != nil {
				return At(fmt.Sprintf("[%d]", i), err)
			}
		}
		return nil
	default:
		return e.Encode(v.Interface())
	}
}

// MsgpackTuple checks an external union tag and its exact positional arity.
func MsgpackTuple(data []byte, tag string, count int) ([][]byte, error) {
	obj, err := MsgpackObject(data)
	if err != nil {
		return nil, err
	}
	raw, ok := obj[tag]
	if len(obj) != 1 || !ok {
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
	r := bytes.NewReader(data)
	d := msgpack.NewDecoder(r)
	id, n, err := d.DecodeExtHeader()
	if err != nil {
		return 0, 0, err
	}
	if id != -1 || n != 4 && n != 8 && n != 12 || r.Len() != n {
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

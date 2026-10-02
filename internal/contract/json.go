// Package contract supplies the generic codec operations used by generated
// contracts. Domain identifiers and timestamps belong to their domain packages;
// this package checks their encodings without defining domain data types.
package contract

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"regexp"
	"strconv"
	"time"
	"unicode/utf8"
)

// Error identifies the path at which a contract was refused.
type Error struct {
	Path string
	Err  error
}

func (e *Error) Error() string {
	if e.Path == "" {
		return e.Err.Error()
	}
	return e.Path + ": " + e.Err.Error()
}
func (e *Error) Unwrap() error { return e.Err }

// At adds a field or array index to an error's path.
func At(path string, err error) error {
	if err == nil {
		return nil
	}
	var nested *Error
	if errors.As(err, &nested) {
		separator := "."
		if path == "" || nested.Path == "" || len(nested.Path) > 0 && nested.Path[0] == '[' {
			separator = ""
		}
		return &Error{Path: path + separator + nested.Path, Err: nested.Err}
	}
	return &Error{Path: path, Err: err}
}

// IsNull reports an explicit JSON null.
func IsNull(data []byte) bool { return bytes.Equal(bytes.TrimSpace(data), []byte("null")) }

// CheckJSON rejects ambiguous objects and malformed Unicode before decoding.
func CheckJSON(data []byte) error {
	if !utf8.Valid(data) {
		return errors.New("invalid UTF-8")
	}
	if err := checkSurrogates(data); err != nil {
		return err
	}
	d := json.NewDecoder(bytes.NewReader(data))
	d.UseNumber()
	if err := scanJSON(d, 0); err != nil {
		return err
	}
	if _, err := d.Token(); !errors.Is(err, io.EOF) {
		if err != nil {
			return err
		}
		return errors.New("trailing JSON data")
	}
	return nil
}
func scanJSON(d *json.Decoder, depth int) error {
	token, err := d.Token()
	if err != nil {
		return err
	}
	delim, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	// serde_json 1.0.151 de.rs starts remaining_depth at 128;
	// check_recursion! decrements before entering an array/object and rejects
	// zero. Thus 127 open containers are accepted, and the 128th is refused.
	if depth >= 127 {
		return errors.New("JSON recursion limit exceeded (128)")
	}
	switch delim {
	case '{':
		seen := map[string]bool{}
		for d.More() {
			token, err := d.Token()
			if err != nil {
				return err
			}
			key, ok := token.(string)
			if !ok {
				return errors.New("expected object key")
			}
			if seen[key] {
				return At(key, errors.New("duplicate key"))
			}
			seen[key] = true
			if err := scanJSON(d, depth+1); err != nil {
				return At(key, err)
			}
		}
	case '[':
		for i := 0; d.More(); i++ {
			if err := scanJSON(d, depth+1); err != nil {
				return At(fmt.Sprintf("[%d]", i), err)
			}
		}
	default:
		return errors.New("unexpected delimiter")
	}
	_, err = d.Token()
	return err
}

// Decode reads a non-null JSON value after checking the entire input.
func Decode[T any](data []byte) (T, error) {
	var value T
	if err := CheckJSON(data); err != nil {
		return value, err
	}
	if IsNull(data) {
		return value, errors.New("null is not allowed")
	}
	err := json.Unmarshal(data, &value)
	return value, err
}

// Object reads exact property names and retains presence independently of null.
func Object(data []byte) (map[string]json.RawMessage, error) {
	return Decode[map[string]json.RawMessage](data)
}

// List applies a generated decoder to each element.
func List[T any](data []byte, decode func([]byte) (T, error)) ([]T, error) {
	raw, err := Decode[[]json.RawMessage](data)
	if err != nil {
		return nil, err
	}
	out := make([]T, len(raw))
	for i, item := range raw {
		out[i], err = decode(item)
		if err != nil {
			return nil, At(fmt.Sprintf("[%d]", i), err)
		}
	}
	return out, nil
}

// Pointer retains a present zero value for an optional field.
func Pointer[T any](data []byte, decode func([]byte) (T, error)) (*T, error) {
	value, err := decode(data)
	if err != nil {
		return nil, err
	}
	return &value, nil
}

// Record decodes a string-keyed record, retaining nullable pointer values.
func Record[T any](data []byte, decode func([]byte) (T, error), nullable bool) (map[string]T, error) {
	raw, err := Object(data)
	if err != nil {
		return nil, err
	}
	out := make(map[string]T, len(raw))
	for key, item := range raw {
		var value T
		if !nullable || !IsNull(item) {
			value, err = decode(item)
			if err != nil {
				return nil, At(key, err)
			}
		}
		out[key] = value
	}
	return out, nil
}

// Text validates lengths in Unicode scalar values and a generation-checked pattern.
func Text(value string, minimum, maximum int, pattern string) error {
	if !utf8.ValidString(value) {
		return errors.New("invalid UTF-8")
	}
	n := utf8.RuneCountInString(value)
	if n < minimum || maximum >= 0 && n > maximum {
		return errors.New("string length outside bounds")
	}
	if pattern != "" {
		matched, err := regexp.MatchString(pattern, value)
		if err != nil {
			return err
		}
		if !matched {
			return errors.New("pattern mismatch")
		}
	}
	return nil
}

// Timestamp checks the canonical UTC millisecond representation.
func Timestamp(value string) error {
	const layout = "2006-01-02T15:04:05.000Z"
	parsed, err := time.Parse(layout, value)
	if err != nil {
		return err
	}
	if parsed.Format(layout) != value {
		return errors.New("expected UTC with three fractional digits")
	}
	return nil
}

// Base64 checks canonical padded standard base64 without whitespace.
func Base64(value string) error {
	decoded, err := base64.StdEncoding.Strict().DecodeString(value)
	if err != nil {
		return err
	}
	if base64.StdEncoding.EncodeToString(decoded) != value {
		return errors.New("noncanonical base64")
	}
	return nil
}

// checkSurrogates prevents encoding/json from replacing malformed escaped Unicode.
func checkSurrogates(data []byte) error {
	inString := false
	for i := 0; i < len(data); i++ {
		if data[i] == '"' {
			inString = !inString
			continue
		}
		if !inString || data[i] != '\\' {
			continue
		}
		i++
		if i >= len(data) || data[i] != 'u' {
			continue
		}
		if i+4 >= len(data) {
			return errors.New("incomplete Unicode escape")
		}
		code, err := strconv.ParseUint(string(data[i+1:i+5]), 16, 16)
		if err != nil {
			return fmt.Errorf("unicode escape: %w", err)
		}
		i += 4
		if code >= 0xdc00 && code <= 0xdfff {
			return errors.New("unpaired low surrogate")
		}
		if code < 0xd800 || code > 0xdbff {
			continue
		}
		if i+6 >= len(data) || data[i+1] != '\\' || data[i+2] != 'u' {
			return errors.New("unpaired high surrogate")
		}
		low, err := strconv.ParseUint(string(data[i+3:i+7]), 16, 16)
		if err != nil {
			return fmt.Errorf("unicode escape: %w", err)
		}
		if low < 0xdc00 || low > 0xdfff {
			return errors.New("unpaired high surrogate")
		}
		i += 6
	}
	return nil
}

// Bytes decodes canonical base64 JSON into a binary field.
func Bytes(data []byte) ([]byte, error) {
	value, err := Decode[string](data)
	if err != nil {
		return nil, err
	}
	if err := Base64(value); err != nil {
		return nil, err
	}
	return base64.StdEncoding.Strict().DecodeString(value)
}

// JSON retains an arbitrary JSON value, including null, after boundary checks.
func JSON(data []byte) (json.RawMessage, error) {
	if err := CheckJSON(data); err != nil {
		return nil, err
	}
	return bytes.Clone(data), nil
}

// Field is one codec property in declaration order.
type Field struct {
	Name  string
	Value any
}

// EncodeObject preserves generated field order, including flattened properties.
func EncodeObject(fields []Field) ([]byte, error) {
	var out bytes.Buffer
	out.WriteByte('{')
	for i, field := range fields {
		if i > 0 {
			out.WriteByte(',')
		}
		key, err := EncodeJSON(field.Name)
		if err != nil {
			return nil, At(field.Name, err)
		}
		value, err := EncodeJSON(field.Value)
		if err != nil {
			return nil, At(field.Name, err)
		}
		out.Write(key)
		out.WriteByte(':')
		out.Write(value)
	}
	out.WriteByte('}')
	return out.Bytes(), nil
}

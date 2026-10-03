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

// ErrSyntax identifies malformed JSON text, including invalid Unicode, trailing
// data and excessive nesting. Shape and value validation errors do not wrap it.
var ErrSyntax = errors.New("invalid JSON syntax")

// Error identifies the path at which a contract was refused.
type Error struct {
	// Path identifies the refused field or element.
	Path string
	// Err is the underlying refusal.
	Err error
}

// Error describes the refused path and its cause.
func (e *Error) Error() string {
	if e.Path == "" {
		return e.Err.Error()
	}
	return e.Path + ": " + e.Err.Error()
}

// Unwrap returns the cause of the contract refusal.
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
		return fmt.Errorf("%w: invalid UTF-8", ErrSyntax)
	}
	if err := checkSurrogates(data); err != nil {
		return fmt.Errorf("%w: %w", ErrSyntax, err)
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := scanJSON(decoder, 0); err != nil {
		return err
	}
	if _, err := decoder.Token(); !errors.Is(err, io.EOF) {
		if err != nil {
			return fmt.Errorf("%w: %w", ErrSyntax, err)
		}
		return fmt.Errorf("%w: trailing JSON data", ErrSyntax)
	}
	return nil
}

func scanJSON(decoder *json.Decoder, depth int) error {
	token, err := decoder.Token()
	if err != nil {
		return fmt.Errorf("%w: %w", ErrSyntax, err)
	}
	delim, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	// A document nests at most 127 arrays and objects; the 128th open one is
	// refused with "JSON recursion limit exceeded (128)".
	if depth >= 127 {
		return fmt.Errorf("%w: JSON recursion limit exceeded (128)", ErrSyntax)
	}
	switch delim {
	case '{':
		seen := map[string]bool{}
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return fmt.Errorf("%w: %w", ErrSyntax, err)
			}
			key, ok := token.(string)
			if !ok {
				return fmt.Errorf("%w: expected object key", ErrSyntax)
			}
			if seen[key] {
				return At(key, errors.New("duplicate key"))
			}
			seen[key] = true
			if err := scanJSON(decoder, depth+1); err != nil {
				return At(key, err)
			}
		}
	case '[':
		for i := 0; decoder.More(); i++ {
			if err := scanJSON(decoder, depth+1); err != nil {
				return At(fmt.Sprintf("[%d]", i), err)
			}
		}
	default:
		return fmt.Errorf("%w: unexpected delimiter", ErrSyntax)
	}
	_, err = decoder.Token()
	if err != nil {
		return fmt.Errorf("%w: %w", ErrSyntax, err)
	}
	return nil
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
	output := make([]T, len(raw))
	for i, item := range raw {
		output[i], err = decode(item)
		if err != nil {
			return nil, At(fmt.Sprintf("[%d]", i), err)
		}
	}
	return output, nil
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
	return KeyedRecord[string](data, decode, nullable)
}

// KeyedRecord retains named string keys; generated validation checks each key.
func KeyedRecord[K ~string, T any](data []byte, decode func([]byte) (T, error), nullable bool) (map[K]T, error) {
	raw, err := Object(data)
	if err != nil {
		return nil, err
	}
	output := make(map[K]T, len(raw))
	for key, item := range raw {
		var value T
		if !nullable || !IsNull(item) {
			value, err = decode(item)
			if err != nil {
				return nil, At(key, err)
			}
		}
		output[K(key)] = value
	}
	return output, nil
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
	// Name is the property name.
	Name string
	// Value is the property value.
	Value any
}

// EncodeObject preserves generated field order, including flattened properties.
func EncodeObject(fields []Field) ([]byte, error) {
	var output bytes.Buffer
	output.WriteByte('{')
	for i, field := range fields {
		if i > 0 {
			output.WriteByte(',')
		}
		key, err := EncodeJSON(field.Name)
		if err != nil {
			return nil, At(field.Name, err)
		}
		value, err := EncodeJSON(field.Value)
		if err != nil {
			return nil, At(field.Name, err)
		}
		output.Write(key)
		output.WriteByte(':')
		output.Write(value)
	}
	output.WriteByte('}')
	return output.Bytes(), nil
}

package provider

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"reflect"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// WireError identifies the field of a malformed vendor payload.
type WireError struct {
	Field string
	Err   error
}

// Error returns the diagnostic for this failure.
func (e *WireError) Error() string {
	return e.Field + ": " + e.Err.Error()
}

// Unwrap returns the underlying cause.
func (e *WireError) Unwrap() error { return e.Err }

// Path returns the offending field path, or a dot for the payload itself.
func (e *WireError) Path() string { return e.Field }

// DecodeUntagged reads only declared vendor fields. Nonpointer fields are
// required unless tagged wire:"optional". Unknown fields are ignored.
func DecodeUntagged[T any](text string) (T, error) {
	var value T
	err := decodeVendor([]byte(text), reflect.ValueOf(&value).Elem(), ".")
	return value, err
}

// DecodeTagged reads the type first; ok is false for an unregistered type.
// Payload decoders are shared with nested tagged values.
func DecodeTagged[T any](text string, payloads map[string]func(string) (T, error)) (T, bool, error) {
	// Only the first JSON value is read; any input after it is ignored.
	var zero T
	var first json.RawMessage
	if err := json.NewDecoder(strings.NewReader(text)).Decode(&first); err != nil {
		return zero, false, &WireError{Field: ".", Err: err}
	}
	text = string(first)
	tag, err := DecodeUntagged[struct {
		Type string `json:"type"`
	}](text)
	if err != nil {
		return zero, false, err
	}
	decode, ok := payloads[tag.Type]
	if !ok {
		return zero, false, nil
	}
	value, err := decode(text)
	if err != nil {
		return zero, false, err
	}
	return value, true, nil
}

// Reported reads a field used only to label or report: other shapes mean absent.
type Reported[T any] struct{ Value *T }

// UnmarshalJSON treats a value of the wrong shape as absent.
func (r *Reported[T]) UnmarshalJSON(data []byte) error {
	canonical, err := canonicalVendorJSON(data, 0)
	if err != nil {
		return err
	}
	value, err := DecodeUntagged[T](string(canonical))
	r.Value = nil
	if err == nil {
		r.Value = &value
	}
	return nil
}

// ReportedString is a vendor-reported text field.
type ReportedString = Reported[string]

// NonEmpty is vendor text that must not be empty.
type NonEmpty string

// UnmarshalJSON requires a nonempty JSON string.
func (n *NonEmpty) UnmarshalJSON(data []byte) error {
	text, err := DecodeUntagged[string](string(data))
	if err != nil {
		return err
	}
	if text == "" {
		return errors.New("expected a nonempty string")
	}
	*n = NonEmpty(text)
	return nil
}

// decodeVendor validates presence, exact field names and scalar kinds without
// imposing Demi's strict unknown-field rules on vendor objects.
func decodeVendor(data []byte, target reflect.Value, path string) error {
	fail := func(err error) error { return &WireError{Field: path, Err: err} }
	if target.Kind() == reflect.Pointer {
		if contract.IsNull(data) {
			return nil
		}
		target.Set(reflect.New(target.Type().Elem()))
		return decodeVendor(data, target.Elem(), path)
	}
	if target.CanAddr() {
		if custom, ok := target.Addr().Interface().(json.Unmarshaler); ok {
			if err := custom.UnmarshalJSON(data); err != nil {
				return fail(err)
			}
			return nil
		}
	}
	if contract.IsNull(data) && target.Kind() != reflect.Interface {
		return fail(errors.New("invalid type: null"))
	}
	switch target.Kind() {
	case reflect.String:
		value, err := contract.Decode[string](data)
		if err != nil {
			return fail(err)
		}
		target.SetString(value)
		return nil
	case reflect.Struct:
		return decodeVendorStruct(data, target, path)
	case reflect.Slice, reflect.Array:
		return decodeVendorSequence(data, target, path)
	default:
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.UseNumber()
		if err := decoder.Decode(target.Addr().Interface()); err != nil {
			return fail(err)
		}
		if _, err := decoder.Token(); !errors.Is(err, io.EOF) {
			return fail(errors.New("trailing JSON data"))
		}
		return nil
	}
}

// Vendor supplies a mapper's failure label, record reader and wall clock.
type Vendor struct {
	Label  string
	Reader FailureReader
	Clock  core.Clock
}

// Undecodable reports a malformed vendor frame without a recovery code.
func Undecodable(label string, err error, text string) Failure {
	return ProtocolFailure(fmt.Sprintf("%s API stream sent a frame Demi cannot read: %v", label, err), text)
}

// usageWithCachedInput separates cached tokens from the reported input count.
func usageWithCachedInput(input, output, read, written *uint64) core.TokenUsage {
	var usage core.TokenUsage
	if input != nil {
		usage.InputTokens = *input
	}
	if output != nil {
		usage.OutputTokens = *output
	}
	if read != nil {
		usage.CacheReadTokens = *read
	}
	if written != nil {
		usage.CacheWriteTokens = *written
	}
	usage.InputTokens -= min(usage.InputTokens, usage.CacheReadTokens)
	usage.InputTokens -= min(usage.InputTokens, usage.CacheWriteTokens)
	return usage
}

// toolInput preserves invalid tool JSON as text so the agent can report it.
func toolInput(text string) json.RawMessage {
	if value, err := canonicalVendorJSON([]byte(text), 0); err == nil {
		return value
	}
	encoded, _ := contract.EncodeJSON(text) // A string always encodes.
	return encoded
}

// canonicalVendorJSON keeps JSON object order for replay; a repeated key keeps
// its first position and its last value; malformed strings and nesting are refused.
func canonicalVendorJSON(data []byte, depth int) (json.RawMessage, error) {
	data = bytes.TrimSpace(data)
	if len(data) == 0 {
		return nil, errors.New("expected JSON value")
	}
	switch data[0] {
	case '{':
		if depth >= 127 {
			return nil, errors.New("JSON recursion limit exceeded (128)")
		}
		return canonicalVendorObject(data, depth)
	case '[':
		if depth >= 127 {
			return nil, errors.New("JSON recursion limit exceeded (128)")
		}
		var parts []json.RawMessage
		if err := json.Unmarshal(data, &parts); err != nil {
			return nil, err
		}
		for i := range parts {
			value, err := canonicalVendorJSON(parts[i], depth+1)
			if err != nil {
				return nil, err
			}
			parts[i] = value
		}
		return contract.EncodeJSON(parts)
	case '"':
		value, err := contract.Decode[string](data)
		if err != nil {
			return nil, err
		}
		return contract.EncodeJSON(value)
	default:
		if !json.Valid(data) {
			return nil, errors.New("invalid JSON value")
		}
		// JSON booleans and null are already canonical. A number is an integer when
		// it fits in 64 bits and otherwise a finite float64.
		if data[0] == 't' || data[0] == 'f' || data[0] == 'n' {
			return data, nil
		}
		return canonicalVendorNumber(data)
	}
}

func decodeVendorStruct(data []byte, target reflect.Value, path string) error {
	fail := func(err error) error { return &WireError{Field: path, Err: err} }
	fields := make(map[string]json.RawMessage)
	known := make(map[string]bool)
	for i := 0; i < target.NumField(); i++ {
		field := target.Type().Field(i)
		if !field.IsExported() {
			continue
		}
		name := strings.Split(field.Tag.Get("json"), ",")[0]
		if name == "" {
			name = field.Name
		}
		if name != "-" {
			known[name] = true
		}
	}
	if err := vendorMembers(data, func(name string, raw json.RawMessage) error {
		if !known[name] {
			return nil
		}
		if _, exists := fields[name]; exists {
			return fmt.Errorf("duplicate field %s", name)
		}
		fields[name] = raw
		return nil
	}); err != nil {
		return fail(err)
	}
	return decodeVendorFields(fields, target, path)
}

func decodeVendorSequence(data []byte, target reflect.Value, path string) error {
	fail := func(err error) error { return &WireError{Field: path, Err: err} }
	// RawMessage implements Unmarshaler and is handled above.
	var values []json.RawMessage
	if err := json.Unmarshal(data, &values); err != nil {
		return fail(err)
	}
	if target.Kind() == reflect.Array {
		if len(values) != target.Len() {
			return fail(errors.New("wrong array length"))
		}
	} else {
		target.Set(reflect.MakeSlice(target.Type(), len(values), len(values)))
	}
	for i, value := range values {
		if err := decodeVendor(
			value,
			target.Index(i),
			fmt.Sprintf("%s[%d]", strings.TrimPrefix(path, "."), i),
		); err != nil {
			return err
		}
	}
	return nil
}

func canonicalVendorObject(data []byte, depth int) (json.RawMessage, error) {
	fields := make([]contract.Field, 0)
	err := vendorMembers(data, func(name string, raw json.RawMessage) error {
		value, err := canonicalVendorJSON(raw, depth+1)
		if err != nil {
			return err
		}
		for i := range fields {
			if fields[i].Name == name {
				fields[i].Value = value
				return nil
			}
		}
		fields = append(fields, contract.Field{Name: name, Value: value})
		return nil
	})
	if err != nil {
		return nil, err
	}
	return contract.EncodeObject(fields)
}

func canonicalVendorNumber(data []byte) (json.RawMessage, error) {
	text := string(data)
	if text != "-0" && !strings.ContainsAny(text, ".eE") {
		if value, err := strconv.ParseInt(text, 10, 64); err == nil {
			return contract.EncodeJSON(value)
		}
		if value, err := strconv.ParseUint(text, 10, 64); err == nil {
			return contract.EncodeJSON(value)
		}
	}
	value, err := strconv.ParseFloat(text, 64)
	if err != nil || math.IsInf(value, 0) {
		return nil, errors.New("number out of range")
	}
	// A float is written in fixed notation for decimal exponents -5 through 15,
	// with ".0" on a whole number, and in exponent notation otherwise.
	mantissa, exp, _ := strings.Cut(strconv.FormatFloat(value, 'e', -1, 64), "e")
	exponent, err := strconv.Atoi(exp)
	if err != nil {
		return nil, err
	}
	number := mantissa + "e" + fmt.Sprintf("%+d", exponent)
	if exponent >= -5 && exponent <= 15 {
		number = strconv.FormatFloat(value, 'f', -1, 64)
		if !strings.Contains(number, ".") {
			number += ".0"
		}
	}
	return contract.EncodeJSON(json.Number(number))
}

func decodeVendorFields(fields map[string]json.RawMessage, target reflect.Value, path string) error {
	for i := 0; i < target.NumField(); i++ {
		field := target.Type().Field(i)
		if !field.IsExported() {
			continue
		}
		name := strings.Split(field.Tag.Get("json"), ",")[0]
		if name == "-" {
			continue
		}
		if name == "" {
			name = field.Name
		}
		raw, found := fields[name]
		next := name
		if path != "." {
			next = path + "." + name
		}
		if !found {
			if field.Type.Kind() == reflect.Pointer || field.Tag.Get("wire") == "optional" {
				continue
			}
			return &WireError{Field: path, Err: fmt.Errorf("missing field %s", name)}
		}
		if err := decodeVendor(raw, target.Field(i), next); err != nil {
			return err
		}
	}
	return nil
}

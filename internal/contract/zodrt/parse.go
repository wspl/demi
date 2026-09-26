package zodrt

import (
	"math"
	"slices"
	"strings"
	"time"
)

// maxSafeInteger is JavaScript's Number.MAX_SAFE_INTEGER.
const maxSafeInteger = 1<<53 - 1

// String accepts a string.
func String(value any) (string, error) {
	text, ok := value.(string)
	if !ok {
		return "", Invalid("expected string, received %s", describe(value))
	}
	return text, nil
}

// Bool accepts a boolean.
func Bool(value any) (bool, error) {
	flag, ok := value.(bool)
	if !ok {
		return false, Invalid("expected boolean, received %s", describe(value))
	}
	return flag, nil
}

// Number accepts a finite number, as `z.number()` does.
func Number(value any) (float64, error) {
	switch number := value.(type) {
	case float64:
		if err := CheckFinite(number); err != nil {
			return 0, err
		}
		return number, nil
	case int64:
		return float64(number), nil
	}
	return 0, Invalid("expected number, received %s", describe(value))
}

// Int accepts a number that is a safe integer, as `z.number().int()` does.
func Int(value any) (int64, error) {
	switch number := value.(type) {
	case float64:
		if math.Trunc(number) != number || math.Abs(number) > maxSafeInteger {
			return 0, Invalid("expected a safe integer, received %v", number)
		}
		return int64(number), nil
	case int64:
		if err := CheckSafeInt(number); err != nil {
			return 0, err
		}
		return number, nil
	}
	return 0, Invalid("expected number, received %s", describe(value))
}

// Bytes accepts binary data (a Uint8Array in TypeScript).
func Bytes(value any) ([]byte, error) {
	data, ok := value.([]byte)
	if !ok {
		return nil, Invalid("expected bytes, received %s", describe(value))
	}
	return data, nil
}

// Date accepts a time (a Date in TypeScript).
func Date(value any) (time.Time, error) {
	instant, ok := value.(time.Time)
	if !ok {
		return time.Time{}, Invalid("expected date, received %s", describe(value))
	}
	return instant, nil
}

// Unknown accepts any value, as `z.unknown()` does.
func Unknown(value any) (any, error) {
	return value, nil
}

// ParseNull accepts null.
func ParseNull(value any) (Null, error) {
	if value != nil {
		return Null{}, Invalid("expected null, received %s", describe(value))
	}
	return Null{}, nil
}

// Array accepts an array whose items the item parser accepts.
func Array[T any](item Parser[T]) Parser[[]T] {
	return func(value any) ([]T, error) {
		items, ok := value.([]any)
		if !ok {
			return nil, Invalid("expected array, received %s", describe(value))
		}
		out := make([]T, len(items))
		for index, entry := range items {
			parsed, err := item(entry)
			if err != nil {
				return nil, AtIndex(index, err)
			}
			out[index] = parsed
		}
		return out, nil
	}
}

// Record accepts an object whose keys and values the parsers accept.
func Record[K ~string, V any](key Parser[K], item Parser[V]) Parser[map[K]V] {
	return func(value any) (map[K]V, error) {
		entries, err := objectEntries(value)
		if err != nil {
			return nil, err
		}
		out := make(map[K]V, len(entries))
		for name, entry := range entries {
			parsedKey, err := key(name)
			if err != nil {
				return nil, At(name, Invalid("invalid key: %s", err))
			}
			parsed, err := item(entry)
			if err != nil {
				return nil, At(name, err)
			}
			out[parsedKey] = parsed
		}
		return out, nil
	}
}

// NullableOf accepts null or what the inner parser accepts.
func NullableOf[T any](inner Parser[T]) Parser[Nullable[T]] {
	return func(value any) (Nullable[T], error) {
		if value == nil {
			return Nullable[T]{}, nil
		}
		parsed, err := inner(value)
		if err != nil {
			return Nullable[T]{}, err
		}
		return NotNull(parsed), nil
	}
}

// Pointer accepts what the inner parser accepts and returns its address; a
// type that contains itself holds itself by pointer.
func Pointer[T any](inner Parser[T]) Parser[*T] {
	return func(value any) (*T, error) {
		parsed, err := inner(value)
		if err != nil {
			return nil, err
		}
		return &parsed, nil
	}
}

// Try parses a union option and checks its constraints, so that a union
// takes an option only when the whole option accepts the value.
func Try[T Validator](parse Parser[T], value any) (T, error) {
	parsed, err := parse(value)
	if err != nil {
		var zero T
		return zero, err
	}
	if err := parsed.Validate(); err != nil {
		var zero T
		return zero, err
	}
	return parsed, nil
}

// FirstOf returns the first candidate that accepts the value, trying them in
// order as a Zod union tries its options.
func FirstOf[T any](value any, candidates ...Parser[T]) (T, error) {
	errs := make([]error, 0, len(candidates))
	for _, candidate := range candidates {
		parsed, err := candidate(value)
		if err == nil {
			return parsed, nil
		}
		errs = append(errs, err)
	}
	var zero T
	return zero, NoMatch(errs)
}

// ObjectMode is how an object treats keys its schema does not name.
type ObjectMode int

const (
	// Strip drops unknown keys (`z.object`).
	Strip ObjectMode = iota
	// Strict rejects unknown keys (`z.strictObject`, `.strict()`).
	Strict
)

// Fields accepts an object and returns its fields. In strict mode a key the
// schema does not name rejects the object.
func Fields(value any, mode ObjectMode, keys []string) (map[string]any, error) {
	entries, err := objectEntries(value)
	if err != nil {
		return nil, err
	}
	if mode == Strict {
		var unknown []string
		for name := range entries {
			if !slices.Contains(keys, name) {
				unknown = append(unknown, name)
			}
		}
		if len(unknown) > 0 {
			slices.Sort(unknown)
			return nil, Invalid("unrecognized keys: %s", strings.Join(unknown, ", "))
		}
	}
	return entries, nil
}

// Required parses a field the object must have.
func Required[T any](fields map[string]any, key string, parse Parser[T]) (T, error) {
	value, ok := fields[key]
	if !ok {
		var zero T
		return zero, At(key, Invalid("required"))
	}
	parsed, err := parse(value)
	if err != nil {
		var zero T
		return zero, At(key, err)
	}
	return parsed, nil
}

// OptionalField parses a field the object may omit. A present field must hold
// a value the parser accepts; null is such a value only for a nullable parser.
func OptionalField[T any](fields map[string]any, key string, parse Parser[T]) (Optional[T], error) {
	value, ok := fields[key]
	if !ok {
		return Optional[T]{}, nil
	}
	parsed, err := parse(value)
	if err != nil {
		return Optional[T]{}, At(key, err)
	}
	return Some(parsed), nil
}

// Constant checks a required literal field, which a generated struct does not
// store because its value is fixed.
func Constant(fields map[string]any, key string, want any) error {
	value, ok := fields[key]
	if !ok {
		return At(key, Invalid("required"))
	}
	if number, isInt := value.(int64); isInt {
		value = float64(number)
	}
	if value != want {
		return At(key, Invalid("expected %v", want))
	}
	return nil
}

// Tag reads a union's string discriminator without judging the value.
func Tag(value any, key string) string {
	entries, err := objectEntries(value)
	if err != nil {
		return ""
	}
	tag, _ := entries[key].(string)
	return tag
}

func objectEntries(value any) (map[string]any, error) {
	switch entries := value.(type) {
	case map[string]any:
		return entries, nil
	case Object:
		out := make(map[string]any, len(entries))
		for _, field := range entries {
			out[field.Key] = field.Value
		}
		return out, nil
	}
	return nil, Invalid("expected object, received %s", describe(value))
}

func describe(value any) string {
	switch value.(type) {
	case nil:
		return "null"
	case bool:
		return "boolean"
	case float64, int64:
		return "number"
	case string:
		return "string"
	case []byte:
		return "bytes"
	case time.Time:
		return "date"
	case []any:
		return "array"
	case map[string]any, Object:
		return "object"
	}
	return "unsupported value"
}

// Package zodrt is the runtime of the Go contracts generated from Demi's Zod
// schemas (scripts/go-zod.ts). It carries no schema of its own: it holds the
// value types generated code shares, the shape checks every schema kind
// needs, and the JSON and MessagePack codecs.
//
// A contract value travels in three forms:
//
//	bytes  --codec decode-->  tree  --generated Parse-->  typed value
//	typed value  --Validate, ToValue-->  tree  --codec encode-->  bytes
//
// The tree is what a Zod schema parses in TypeScript: nil, bool, float64
// (every number, as in JavaScript), string, []byte, time.Time, []any and
// map[string]any from a decoder; generated ToValue adds int64 for integers and
// Object for fields in schema order.
package zodrt

// Optional is an object field the schema lets the sender omit (`.optional()`).
// It is never null: a present field holds a value of T.
type Optional[T any] struct {
	Value   T
	Present bool
}

// Some returns a present optional field.
func Some[T any](value T) Optional[T] {
	return Optional[T]{Value: value, Present: true}
}

// Nullable is a value the schema lets be null (`.nullable()`). Its zero value
// is null.
type Nullable[T any] struct {
	Value T
	Valid bool
}

// NotNull returns a non-null value.
func NotNull[T any](value T) Nullable[T] {
	return Nullable[T]{Value: value, Valid: true}
}

// Null is the value of a `z.null()` schema.
type Null struct{}

// Field is one field of an Object.
type Field struct {
	Key   string
	Value any
}

// Object is an encoded object whose fields keep the order of its schema, as
// Zod's parse output does.
type Object []Field

// Parser converts a decoded tree into a typed value.
type Parser[T any] func(value any) (T, error)

// Validator is a typed contract value that can check its constraints.
type Validator interface {
	Validate() error
}

// ArrayValue encodes the items of a slice.
func ArrayValue[T any](items []T, encode func(T) any) []any {
	out := make([]any, len(items))
	for index, item := range items {
		out[index] = encode(item)
	}
	return out
}

// RecordValue encodes the values of a record.
func RecordValue[K ~string, V any](record map[K]V, encode func(V) any) map[string]any {
	out := make(map[string]any, len(record))
	for key, value := range record {
		out[string(key)] = encode(value)
	}
	return out
}

// NullableValue encodes a nullable value: nil for null.
func NullableValue[T any](value Nullable[T], encode func(T) any) any {
	if !value.Valid {
		return nil
	}
	return encode(value.Value)
}

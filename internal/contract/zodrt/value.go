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
// Object (keys in the order they arrived) from a decoder; generated ToValue
// adds int64 for integers. A tree a Go caller builds may also hold
// map[string]any, whose keys are encoded sorted because a Go map has no order.
package zodrt

import "iter"

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

// Object is an object whose fields keep their order: the order they arrived
// in, or the order of the schema, as Zod's parse output keeps them. The
// encoders write it in JavaScript's property order (see jsPropertyOrder).
type Object []Field

// Record is a Zod record: its keys keep the order they were set in, as a
// JavaScript object keeps them. The zero value is an empty record.
type Record[K ~string, V any] struct {
	keys   []K
	values map[K]V
}

// Set sets a key's value; a new key goes after the existing ones.
func (r *Record[K, V]) Set(key K, value V) {
	if r.values == nil {
		r.values = make(map[K]V)
	}
	if _, ok := r.values[key]; !ok {
		r.keys = append(r.keys, key)
	}
	r.values[key] = value
}

// Get returns a key's value.
func (r Record[K, V]) Get(key K) (V, bool) {
	value, ok := r.values[key]
	return value, ok
}

// Len is the number of keys.
func (r Record[K, V]) Len() int {
	return len(r.keys)
}

// All yields the entries in key order.
func (r Record[K, V]) All() iter.Seq2[K, V] {
	return func(yield func(K, V) bool) {
		for _, key := range r.keys {
			if !yield(key, r.values[key]) {
				return
			}
		}
	}
}

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

// RecordValue encodes a record, keys in their order.
func RecordValue[K ~string, V any](record Record[K, V], encode func(V) any) Object {
	out := make(Object, 0, record.Len())
	for key, value := range record.All() {
		out = append(out, Field{Key: string(key), Value: encode(value)})
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

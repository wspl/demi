package zodrt

import (
	"cmp"
	"slices"
	"strconv"
)

// MaxDepth is the deepest nesting of arrays and objects a decoder accepts,
// encoding/json's limit. A deeper value is refused before it is walked, so a
// small frame cannot exhaust the stack.
const MaxDepth = 10_000

// setField sets a decoded object's field as a JavaScript object does: a
// repeated key keeps its first position and takes its last value.
func setField(fields Object, positions map[string]int, key string, value any) Object {
	if position, ok := positions[key]; ok {
		fields[position].Value = value
		return fields
	}
	positions[key] = len(fields)
	return append(fields, Field{Key: key, Value: value})
}

// jsPropertyOrder is the order in which JavaScript enumerates an object's
// keys, which JSON.stringify and @msgpack/msgpack write: array-index keys
// ("0", "10") ascending first, then the others in insertion order.
func jsPropertyOrder(fields Object) Object {
	if !slices.ContainsFunc(fields, func(field Field) bool { return isArrayIndex(field.Key) }) {
		return fields
	}
	ordered := make(Object, 0, len(fields))
	for _, field := range fields {
		if isArrayIndex(field.Key) {
			ordered = append(ordered, field)
		}
	}
	slices.SortFunc(ordered, func(first, second Field) int {
		return cmp.Compare(arrayIndex(first.Key), arrayIndex(second.Key))
	})
	for _, field := range fields {
		if !isArrayIndex(field.Key) {
			ordered = append(ordered, field)
		}
	}
	return ordered
}

// isArrayIndex reports whether a key is a canonical array index: an integer
// from 0 to 2^32-2 written without a sign or leading zeros.
func isArrayIndex(key string) bool {
	if key == "" || (len(key) > 1 && key[0] == '0') {
		return false
	}
	index, err := strconv.ParseUint(key, 10, 64)
	return err == nil && index < 1<<32-1
}

// arrayIndex is the value of a key isArrayIndex accepted.
func arrayIndex(key string) uint64 {
	// isArrayIndex has parsed the key already.
	index, _ := strconv.ParseUint(key, 10, 64)
	return index
}

package cmddecl

import (
	"iter"

	"github.com/wspl/demi/internal/contract"
)

// Arguments holds command values in insertion order. Its zero value is empty.
type Arguments struct {
	fields []contract.Field
}

// Set inserts an argument or replaces its value without changing its position.
func (a *Arguments) Set(name string, value any) {
	for i, field := range a.fields {
		if field.Name == name {
			a.fields[i].Value = value
			return
		}
	}
	a.fields = append(a.fields, contract.Field{Name: name, Value: value})
}

// Lookup returns the argument and whether it exists.
func (a Arguments) Lookup(name string) (any, bool) {
	for _, field := range a.fields {
		if field.Name == name {
			return field.Value, true
		}
	}
	return nil, false
}

// All visits command arguments in insertion order.
func (a Arguments) All() iter.Seq2[string, any] {
	return func(yield func(string, any) bool) {
		for _, field := range a.fields {
			if !yield(field.Name, field.Value) {
				return
			}
		}
	}
}

// MarshalJSON encodes the runtime argument object in insertion order.
func (a Arguments) MarshalJSON() ([]byte, error) {
	return contract.EncodeObject(a.fields)
}

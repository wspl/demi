package main

import (
	"cmp"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// schemaObject keeps keywords and properties in insertion order while
// later field annotations replace existing keywords in place.
type schemaObject struct{ fields []contract.Field }

func (s *schemaObject) set(name string, value any) {
	for i := range s.fields {
		if s.fields[i].Name == name {
			s.fields[i].Value = value
			return
		}
	}
	s.fields = append(s.fields, contract.Field{Name: name, Value: value})
}

func (s *schemaObject) get(name string) any {
	for _, field := range s.fields {
		if field.Name == name {
			return field.Value
		}
	}
	return nil
}

func (s *schemaObject) remove(name string) {
	s.fields = slices.DeleteFunc(s.fields, func(field contract.Field) bool { return field.Name == name })
}

func (s *schemaObject) clone() *schemaObject {
	return &schemaObject{fields: slices.Clone(s.fields)}
}

func (s *schemaObject) MarshalJSON() ([]byte, error) {
	return contract.EncodeObject(s.fields)
}

// schemaKeywords starts a schema with keywords in the given order.
func schemaKeywords(fields ...contract.Field) *schemaObject {
	return &schemaObject{fields: fields}
}

// serializedSchema orders every schema object's keywords: $id, $schema, title,
// description, type, format and properties first, the others in insertion order,
// $defs and definitions last. Property names keep insertion order, array items
// are ordered as schemas, and default, examples and x- values are copied as they are.
func serializedSchema(value any, properties bool) any {
	switch value := value.(type) {
	case *schemaObject:
		out := &schemaObject{}
		for _, field := range value.fields {
			child := field.Value
			if properties ||
				field.Name != "default" && field.Name != "examples" && !strings.HasPrefix(field.Name, "x-") {
				noReorder := !properties &&
					slices.Contains(
						[]string{"properties", "patternProperties", "dependentSchemas", "$defs", "definitions"},
						field.Name,
					)
				child = serializedSchema(child, noReorder)
			}
			out.set(field.Name, child)
		}
		if !properties {
			start := []string{"$id", "$schema", "title", "description", "type", "format", "properties"}
			end := []string{"$defs", "definitions"}
			rank := func(name string) int {
				if i := slices.Index(start, name); i >= 0 {
					return i
				}
				if i := slices.Index(end, name); i >= 0 {
					return len(start) + 1 + i
				}
				return len(start)
			}
			slices.SortStableFunc(
				out.fields,
				func(a, b contract.Field) int { return cmp.Compare(rank(a.Name), rank(b.Name)) },
			)
		}
		return out
	case []any:
		out := make([]any, len(value))
		for i, child := range value {
			out[i] = serializedSchema(child, false)
		}
		return out
	default:
		return value
	}
}

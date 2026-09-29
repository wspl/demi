package commandtree

import (
	"bytes"
	// The schema library reads numbers as the v1 package's Number type, which
	// keeps the text a document wrote, and the v2 package encodes it as a number.
	jsonv1 "encoding/json"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"iter"
	"maps"
	"slices"

	"github.com/wspl/demi/go/internal/wire"
)

// An Object is a JSON object whose members keep the order in which they were
// added or read. The order is part of what the model reads: help lists a
// command's parameters in the order its schema declares them, and a check
// reports the failures of an input's members in the order the input has them.
//
// The values of an Object, and of the values that [DecodeValue] returns, are
// nil, bool, string, [encoding/json.Number], int64, float64, []any and Object.
type Object struct {
	names  []string
	values map[string]any
}

// Set adds the member name, or replaces its value and keeps its place.
func (o *Object) Set(name string, value any) {
	if o.values == nil {
		o.values = map[string]any{}
	}
	if _, ok := o.values[name]; !ok {
		o.names = append(o.names, name)
	}
	o.values[name] = value
}

// Get returns the value of the member name.
func (o Object) Get(name string) (any, bool) {
	value, ok := o.values[name]
	return value, ok
}

// Len returns the number of members.
func (o Object) Len() int {
	return len(o.names)
}

// All returns the members in order.
func (o Object) All() iter.Seq2[string, any] {
	return func(yield func(string, any) bool) {
		for _, name := range o.names {
			if !yield(name, o.values[name]) {
				return
			}
		}
	}
}

// position returns the index of the member name in the order of the members,
// or -1.
func (o Object) position(name string) int {
	for i, member := range o.names {
		if member == name {
			return i
		}
	}
	return -1
}

// sortedNames returns the names of the members in the order of their text, as
// the members of a map come.
func (o Object) sortedNames() []string {
	return slices.Sorted(slices.Values(o.names))
}

// sorted returns an Object with the members of o in the order of their names.
func (o Object) sorted() Object {
	var sorted Object
	for _, name := range o.sortedNames() {
		sorted.Set(name, o.values[name])
	}
	return sorted
}

// MarshalJSONTo writes the members in order.
func (o Object) MarshalJSONTo(enc *jsontext.Encoder) error {
	if err := enc.WriteToken(jsontext.BeginObject); err != nil {
		return err
	}
	for name, value := range o.All() {
		if err := enc.WriteToken(jsontext.String(name)); err != nil {
			return err
		}
		if err := json.MarshalEncode(enc, value); err != nil {
			return err
		}
	}
	return enc.WriteToken(jsontext.EndObject)
}

// UnmarshalJSONFrom reads an object and keeps the order of its members.
func (o *Object) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	*o = Object{}
	if err := wire.BeginObject(dec); err != nil {
		return err
	}
	for {
		name, more, err := wire.NextMember(dec)
		if err != nil {
			return err
		}
		if !more {
			return nil
		}
		value, err := readValue(dec)
		if err != nil {
			return wire.In(name, err)
		}
		o.Set(name, value)
	}
}

// readValue reads one JSON value, with its objects as [Object] and its numbers
// as [jsonv1.Number], which keeps the text the document wrote.
func readValue(dec *jsontext.Decoder) (any, error) {
	switch dec.PeekKind() {
	case '{':
		var object Object
		if err := object.UnmarshalJSONFrom(dec); err != nil {
			return nil, err
		}
		return object, nil
	case '[':
		if err := wire.BeginArray(dec); err != nil {
			return nil, err
		}
		elements := []any{}
		for {
			more, err := wire.NextElement(dec)
			if err != nil {
				return nil, err
			}
			if !more {
				return elements, nil
			}
			element, err := readValue(dec)
			if err != nil {
				return nil, wire.In(wire.Index(len(elements)), err)
			}
			elements = append(elements, element)
		}
	}
	token, err := dec.ReadToken()
	if err != nil {
		return nil, err
	}
	switch token.Kind() {
	case 'n':
		return nil, nil
	case 't', 'f':
		return token.Bool(), nil
	case '"':
		return token.String(), nil
	default:
		return jsonv1.Number(token.String()), nil
	}
}

// A document reads JSON into the values of an [Object].
type document struct {
	value any
}

func (d *document) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	value, err := readValue(dec)
	d.value = value
	return err
}

// DecodeValue decodes one JSON document into the values [Object] holds, so a
// check sees the members of an input in the order the document has them. The
// error says where the document is not valid JSON.
func DecodeValue(data []byte) (any, error) {
	var d document
	if err := json.Unmarshal(data, &d); err != nil {
		return nil, wire.Refusal(err)
	}
	return d.value, nil
}

// encodeValue returns the compact JSON of a value.
func encodeValue(value any) string {
	var out bytes.Buffer
	enc := jsontext.NewEncoder(&out)
	if err := json.MarshalEncode(enc, value); err != nil {
		return ""
	}
	return string(bytes.TrimRight(out.Bytes(), "\n"))
}

// plain returns value as the values of the standard library, which the schema
// library validates: objects become maps.
func plain(value any) any {
	switch value := value.(type) {
	case Object:
		members := make(map[string]any, value.Len())
		for name, member := range value.All() {
			members[name] = plain(member)
		}
		return members
	case []any:
		elements := make([]any, len(value))
		for i, element := range value {
			elements[i] = plain(element)
		}
		return elements
	}
	return value
}

// clone returns an Object with the members of o, which it may change without
// changing o.
func (o Object) clone() Object {
	return Object{names: slices.Clone(o.names), values: maps.Clone(o.values)}
}

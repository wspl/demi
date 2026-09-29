package commandtree

import (
	"cmp"
	"errors"
	"fmt"
	"net/url"
	"slices"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
)

// A failure is one way an instance breaks a schema: where the value is, which
// keyword it breaks, and the library's parameters of that failure. The library
// reports a failure by its kind and never by its message, which is not parsed.
type failure struct {
	// location is the instance's path to the value, one token for each
	// member name or array index.
	location []string
	// pointer is the schema's path to the keyword: the tokens of its JSON
	// pointer from the schema document's root.
	pointer []string
	// place is the keyword's position among the members of its schema object.
	place int
	kind  jsonschema.ErrorKind
	// name is the property a failure of "required" names, one failure for each
	// missing property, and index is the property's place among the missing.
	name  string
	index int
	// unexpected are the properties that "additionalProperties" refuses, in the
	// instance's order.
	unexpected []string
	// literal is the text a message calls the value by, instead of its path,
	// for the failure of a property's name, which the Rust names by its text.
	literal string
	// inner is the first failure of the name a "propertyNames" schema refuses.
	inner *failure
}

// failures returns every failure of the instance, as the library reports them
// and no more: it stops a schema's keywords at the first of type, const, enum
// and format that fails, and the failures are what it found.
//
// The order is one rule, whatever the library's own order: failures sort by the
// instance location they are about (the members of an object in the order the
// instance has them, array elements by index, a value before what it holds),
// then by the place of the failing keyword in its schema object (a schema's
// top-level keywords by name, since a [Schema] holds them sorted; a nested
// schema's in the order it wrote them), then by the keyword's path in the schema, and
// last, for the failures of one keyword, by the property they name.
func (s *Schema) failures(instance any) ([]failure, error) {
	err := s.compiled.Validate(plain(instance))
	if err == nil {
		return nil, nil
	}
	var refusal *jsonschema.ValidationError
	if !errors.As(err, &refusal) {
		return nil, fmt.Errorf("commandtree: cannot check the instance: %w", err)
	}
	var leaves []failure
	s.collect(refusal, &leaves)
	var found []failure
	for _, leaf := range leaves {
		if invalid, ok := leaf.kind.(*kind.InvalidJsonValue); ok {
			// A value that JSON has no form for: a caller's mistake.
			return nil, fmt.Errorf("commandtree: cannot check the instance: it holds a %T", invalid.Value)
		}
		found = append(found, split(leaf, instance)...)
	}
	sortFailures(found, instance)
	return found, nil
}

// sortFailures orders failures by the rule at [Schema.failures].
func sortFailures(found []failure, instance any) {
	slices.SortFunc(found, func(a, b failure) int {
		return cmp.Or(
			slices.Compare(locationRank(a.location, instance), locationRank(b.location, instance)),
			cmp.Compare(a.place, b.place),
			slices.Compare(a.pointer, b.pointer),
			cmp.Compare(a.index, b.index),
		)
	})
}

// locationRank returns the place of each token of a location among its
// siblings in the instance: a member's position in its object, an element's
// index.
func locationRank(location []string, instance any) []int {
	rank := make([]int, len(location))
	value := instance
	for i, token := range location {
		switch container := value.(type) {
		case Object:
			rank[i] = container.position(token)
		case []any:
			rank[i], _ = strconv.Atoi(token)
		}
		value = lookup(value, []string{token})
	}
	return rank
}

// collect appends the failures under one the library reported. A failure that
// only groups others is not one itself.
func (s *Schema) collect(refusal *jsonschema.ValidationError, found *[]failure) {
	switch refusal.ErrorKind.(type) {
	case *kind.Schema, *kind.Group, *kind.Reference, *kind.AllOf:
		for _, cause := range refusal.Causes {
			s.collect(cause, found)
		}
		return
	}
	pointer := pointerOf(refusal)
	f := failure{
		location: refusal.InstanceLocation,
		pointer:  pointer,
		place:    s.place(refusal, pointer),
		kind:     refusal.ErrorKind,
	}
	if names, ok := refusal.ErrorKind.(*kind.PropertyNames); ok {
		// The causes are the failures of the name, a string.
		var inner []failure
		for _, cause := range refusal.Causes {
			s.collect(cause, &inner)
		}
		if len(inner) > 0 {
			inner[0].literal = encodeValue(names.Property)
			f.inner = &inner[0]
		}
	}
	*found = append(*found, f)
}

// place returns the position of the keyword a failure broke among the members
// of its schema object, or the number of members when it cannot be found.
func (s *Schema) place(refusal *jsonschema.ValidationError, pointer []string) int {
	keyword := len(pointer) - len(refusal.ErrorKind.KeywordPath())
	if _, ok := refusal.ErrorKind.(*kind.Not); ok {
		keyword = len(pointer) - 1
	}
	if keyword < 0 || keyword >= len(pointer) {
		return 0
	}
	node, ok := lookup(s.document, pointer[:keyword]).(Object)
	if !ok {
		return 0
	}
	return node.position(pointer[keyword])
}

// pointerOf returns the tokens of the place in the schema document of the
// keyword that a failure broke.
func pointerOf(refusal *jsonschema.ValidationError) []string {
	var tokens []string
	if _, fragment, ok := strings.Cut(refusal.SchemaURL, "#"); ok {
		for _, token := range strings.Split(fragment, "/")[1:] {
			if unescaped, err := url.PathUnescape(token); err == nil {
				token = unescaped
			}
			token = strings.ReplaceAll(token, "~1", "/")
			token = strings.ReplaceAll(token, "~0", "~")
			tokens = append(tokens, token)
		}
	}
	tokens = append(tokens, refusal.ErrorKind.KeywordPath()...)
	if _, ok := refusal.ErrorKind.(*kind.Not); ok {
		tokens = append(tokens, "not")
	}
	return tokens
}

// split returns the failures that one the library reported holds: the Rust
// implementation reports each missing property on its own, and names the
// unexpected properties in the instance's order.
func split(leaf failure, instance any) []failure {
	switch k := leaf.kind.(type) {
	case *kind.Required:
		return named(leaf, k.Missing)
	case *kind.DependentRequired:
		return named(leaf, k.Missing)
	case *kind.Dependency:
		return named(leaf, k.Missing)
	case *kind.AdditionalProperties:
		leaf.unexpected = inOrder(k.Properties, lookup(instance, leaf.location))
	case *kind.PropertyNames:
		if object, ok := lookup(instance, leaf.location).(Object); ok {
			leaf.index = object.position(k.Property)
		}
	}
	return []failure{leaf}
}

// named returns a failure for each missing property, in the order the schema
// names them.
func named(leaf failure, missing []string) []failure {
	found := make([]failure, len(missing))
	for i, name := range missing {
		found[i] = leaf
		found[i].name = name
		found[i].index = i
	}
	return found
}

// inOrder returns names in the order of the members of value, an object. The
// library finds the unexpected properties in the order of a map.
func inOrder(names []string, value any) []string {
	ordered := slices.Clone(names)
	object, ok := value.(Object)
	if !ok {
		slices.Sort(ordered)
		return ordered
	}
	slices.SortStableFunc(ordered, func(a, b string) int {
		return cmp.Compare(object.position(a), object.position(b))
	})
	return ordered
}

// lookup returns the value at the JSON pointer tokens of a document.
func lookup(value any, tokens []string) any {
	for _, token := range tokens {
		switch container := value.(type) {
		case Object:
			member, ok := container.Get(token)
			if !ok {
				return nil
			}
			value = member
		case []any:
			index, err := strconv.Atoi(token)
			if err != nil || index < 0 || index >= len(container) {
				return nil
			}
			value = container[index]
		default:
			return nil
		}
	}
	return value
}

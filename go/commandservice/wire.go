package commandservice

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"strings"

	"github.com/google/jsonschema-go/jsonschema"
)

// Decode checks data, one JSON document from outside the process, and returns
// it as a T, one of the wire's types. It refuses a document over
// [MaxMetadataBytes] with [ErrTooLarge], and one that is not valid JSON, breaks
// the schema of T (a required member missing, an unknown member, a null where a
// value is optional, a constraint of the type) or breaks a rule a schema cannot
// express, with an [*InvalidError]. The error names the field and the rule,
// never the value: a value may be a secret.
func Decode[T any](data []byte) (T, error) {
	var value T
	w, err := wireOf[T]()
	if err != nil {
		return value, err
	}
	if len(data) > MaxMetadataBytes {
		return value, ErrTooLarge
	}
	if err := w.validate(data); err != nil {
		return value, err
	}
	if err := json.Unmarshal(data, &value); err != nil {
		return value, unfit(err)
	}
	if w.check != nil {
		if err := w.check(&value); err != nil {
			return value, &InvalidError{Reason: err.Error()}
		}
	}
	return value, nil
}

// Encode returns the JSON of value, one of the wire's types, after checking it
// as [Decode] would, so the SDK never sends what its peer would refuse.
func Encode[T any](value T) ([]byte, error) {
	w, err := wireOf[T]()
	if err != nil {
		return nil, err
	}
	data, err := json.Marshal(value, json.Deterministic(true))
	if err != nil {
		return nil, unfit(err)
	}
	if len(data) > MaxMetadataBytes {
		return nil, ErrTooLarge
	}
	if err := w.validate(data); err != nil {
		return nil, err
	}
	if w.check != nil {
		if err := w.check(&value); err != nil {
			return nil, &InvalidError{Reason: err.Error()}
		}
	}
	return data, nil
}

// A wire holds what is known about one wire type: its schema, derived from the
// Go type and the constraints written beside the type, and its check of the
// rules a schema cannot express.
type wire struct {
	typ      reflect.Type
	schema   *jsonschema.Schema
	resolved *jsonschema.Resolved
	// check takes a pointer to a value of typ; it is nil when the schema says it
	// all.
	check func(value any) error
}

// wires is the registry of the wire's types, keyed by their Go type: each is
// declared where the type is defined, when the package initializes.
var wires = map[reflect.Type]*wire{}

func wireOf[T any]() (*wire, error) {
	typ := reflect.TypeFor[T]()
	w, ok := wires[typ]
	if !ok {
		return nil, fmt.Errorf("commandservice: %s is not a type of the wire", typ)
	}
	return w, nil
}

// declare builds the schema of T once, when the package initializes, registers
// it for [Decode] and [Encode], and returns it for the wire types that embed T:
// the schema jsonschema infers from the type (a field without omitempty or
// omitzero is required, and an object refuses unknown members), then refine,
// which adds the constraints of the type. parts are the wire types T embeds.
// check runs after decoding, for the rules a schema cannot express; it may be
// nil. A wire type that no other embeds is declared as `var _ = declare[T](...)`.
func declare[T any](refine func(*jsonschema.Schema), check func(*T) error, parts ...*wire) *wire {
	typ := reflect.TypeFor[T]()
	schema := schemaOf[T](parts...)
	if refine != nil {
		refine(schema)
	}
	resolved, err := schema.Resolve(nil)
	if err != nil {
		panic(fmt.Sprintf("commandservice: the schema of %s does not resolve: %v", typ, err))
	}
	w := &wire{typ: typ, schema: schema, resolved: resolved}
	if check != nil {
		// Decode and Encode hand check the pointer to a T that it was declared for.
		w.check = func(value any) error { return check(value.(*T)) }
	}
	wires[typ] = w
	return w
}

// schemaOf infers the schema of T, with no null in it, reusing the schemas of
// the wire types parts that T embeds.
func schemaOf[T any](parts ...*wire) *jsonschema.Schema {
	options := &jsonschema.ForOptions{TypeSchemas: map[reflect.Type]*jsonschema.Schema{}}
	for _, part := range parts {
		options.TypeSchemas[part.typ] = part.schema
	}
	schema, err := jsonschema.For[T](options)
	if err != nil {
		panic(fmt.Sprintf("commandservice: the schema of %s: %v", reflect.TypeFor[T](), err))
	}
	refuseNull(schema)
	return schema
}

// validate checks the JSON document data against the schema. jsonschema
// validates the Go value a document unmarshals into, not its bytes, so the
// document is unmarshaled once as an untyped value for the schema and once,
// after the schema accepts it, into the type.
func (w *wire) validate(data []byte) error {
	var document any
	if err := json.Unmarshal(data, &document); err != nil {
		return unfit(err)
	}
	if err := w.resolved.Validate(document); err != nil {
		return violation(err)
	}
	return nil
}

// violation describes the failure of a schema validation as the schema location
// that a value broke and the rule, and leaves out the value, which jsonschema
// quotes in most of its messages. jsonschema reports a failure as text: a chain
// of errors that each say "validating <location>: " and, last, the rule's own
// message, which starts with the rule's name. The library gives no structured
// error (a location and a rule as values) to read instead, so this reads its
// text; TestARefusalNamesTheFieldAndTheRuleAndNeverTheValue guards the reading.
// A message that names members and no value is kept whole.
func violation(err error) *InvalidError {
	location := "root"
	leaf := err
	for {
		inner := errors.Unwrap(leaf)
		if inner == nil {
			break
		}
		wrapped := strings.TrimSuffix(leaf.Error(), ": "+inner.Error())
		location = strings.TrimPrefix(wrapped, "validating ")
		leaf = inner
	}
	rule := leaf.Error()
	if !isMemberRule(rule) {
		rule, _, _ = strings.Cut(rule, ":")
	}
	return &InvalidError{Reason: location + ": " + rule}
}

// isMemberRule reports whether the message of a jsonschema rule names members
// of an object and nothing else.
func isMemberRule(message string) bool {
	for _, prefix := range []string{"required:", "unexpected additional properties", "minItems:", "maxItems:", "uniqueItems:", "minProperties:", "maxProperties:"} {
		if strings.HasPrefix(message, prefix) {
			return true
		}
	}
	return false
}

// unfit describes why a document could not be decoded from or encoded as JSON
// without quoting what it holds: the JSON pointer of the member and the kind of
// failure.
func unfit(err error) *InvalidError {
	var pointer jsontext.Pointer
	rule := "is not valid JSON"
	var semantic *json.SemanticError
	var syntactic *jsontext.SyntacticError
	switch {
	case errors.As(err, &semantic):
		pointer = semantic.JSONPointer
		rule = "does not fit its type"
	case errors.As(err, &syntactic):
		pointer = syntactic.JSONPointer
	}
	location := "root"
	if pointer != "" {
		location = string(pointer)
	}
	return &InvalidError{Reason: location + ": " + rule}
}

// refuseNull removes the null that jsonschema.For adds to the type of every
// pointer and slice, from s and everything beneath it: no field of the wire is
// nullable, so a null is refused wherever a value is optional. jsonschema
// v0.4.3 has no option to leave the null out.
func refuseNull(s *jsonschema.Schema) {
	if slices.Contains(s.Types, "null") {
		types := slices.DeleteFunc(slices.Clone(s.Types), func(name string) bool { return name == "null" })
		s.Types = nil
		if len(types) == 1 {
			s.Type = types[0]
		} else {
			s.Types = types
		}
	}
	for _, property := range s.Properties {
		refuseNull(property)
	}
	if s.Items != nil {
		refuseNull(s.Items)
	}
	if s.AdditionalProperties != nil {
		refuseNull(s.AdditionalProperties)
	}
}

// prop returns the schema of the property name of the object schema s.
func prop(s *jsonschema.Schema, name string) *jsonschema.Schema {
	property := s.Properties[name]
	if property == nil {
		panic(fmt.Sprintf("commandservice: the schema has no property %q", name))
	}
	return property
}

// The patterns of strings that must not hold a NUL character, as paths and
// environment values must not.
const (
	patternNonEmptyNoNUL = `^[^\x00]+$`
	patternEnvName       = `^[^\x00=]+$`
	patternNoNUL         = `^[^\x00]*$`
)

// limitConversationName adds the rule of a conversation's name to the schema of
// a string: 1 to [ConversationNameChars] ASCII letters, digits, '-' and '_'. The
// name is a conversation's, or the provider entry's that work outside any
// conversation serves. The runner names a job's directory after it, so it can
// never name another path.
func limitConversationName(s *jsonschema.Schema) {
	s.Pattern = fmt.Sprintf(`^[A-Za-z0-9_-]{1,%d}$`, ConversationNameChars)
}

// A form is one shape of a value that has one of two: the members it requires
// and the members it refuses, which the other shape has.
type form struct {
	requires []string
	refuses  []string
}

// rules returns the schema of the form's members.
func (f form) rules() *jsonschema.Schema {
	rules := &jsonschema.Schema{Required: f.requires}
	for _, name := range f.refuses {
		if rules.Properties == nil {
			rules.Properties = map[string]*jsonschema.Schema{}
		}
		// The schema that nothing matches.
		rules.Properties[name] = &jsonschema.Schema{Not: &jsonschema.Schema{}}
	}
	return rules
}

// tagged adds to s, the schema inferred from a struct that stands for a value
// of one of two forms, the rules that tell them apart by the string member tag:
// first when tag is firstValue, second when it is secondValue. A value with
// another tag, or none, is refused for that, by the type's own schema. The
// error of a value that breaks its form is the form's, which names the member
// and the rule; a oneOf would say only that no form fit.
func tagged(s *jsonschema.Schema, tag, firstValue, secondValue string, first, second form) {
	prop(s, tag).Enum = []any{firstValue, secondValue}
	tagIs := func(value string) *jsonschema.Schema {
		return &jsonschema.Schema{
			Required:   []string{tag},
			Properties: map[string]*jsonschema.Schema{tag: {Const: jsonschema.Ptr[any](value)}},
		}
	}
	s.If = tagIs(firstValue)
	s.Then = first.rules()
	s.Else = &jsonschema.Schema{If: tagIs(secondValue), Then: second.rules()}
}

// limitOperations adds the rules of a list of operation names to the schema of
// an array: at least one, each named, none twice.
func limitOperations(s *jsonschema.Schema) {
	s.MinItems = jsonschema.Ptr(1)
	s.Items.MinLength = jsonschema.Ptr(1)
	s.UniqueItems = true
}

// limitLength adds the bounds of a string's length in characters (Unicode
// scalar values, which jsonschema counts) to its schema.
func limitLength(s *jsonschema.Schema, minimum, maximum int) {
	s.MinLength = jsonschema.Ptr(minimum)
	s.MaxLength = jsonschema.Ptr(maximum)
}

// Package wiregen reads the wire types a Go package declares and writes their
// decoders, their rule checks and their TypeScript schemas. Its command is
// cmd/wiregen; the design is docs/internal/go-migration/design/wire-contracts.md.
package wiregen

import (
	"go/constant"
)

// A Package is the wire declarations of one Go package.
type Package struct {
	// Name is the package's name.
	Name string
	// Files are the source files that declare wire types, by name.
	Files []*File
	// Structs and Unions are the marked declarations by name.
	Structs map[string]*Struct
	Unions  map[string]*Union
	// Opaque are the marked structs the package decodes itself: the generated
	// code calls their UnmarshalJSONFrom and generates nothing for them.
	Opaque map[string]bool

	// named are the types the wire types use that are named after a basic type:
	// closed sets and strings with rules (see [Type]).
	named map[string]*Type
	// consts are the values of the package's constants, by name.
	consts map[string]constant.Value
	// patterns are the sources of the package's regular expressions, by the
	// name of their variable.
	patterns map[string]string
}

// A File is a source file with wire declarations, and their names in order.
type File struct {
	// Name is the file's name, such as "invocation.go".
	Name  string
	Types []string
}

// A Struct is a marked struct: a wire type, or the variant of a union.
type Struct struct {
	Name   string
	Doc    string
	Fields []*Field
	// Union is the union the struct is a variant of, or nil.
	Union *Union
	// Tag is the tag value of a variant of a tagged union.
	Tag string
	// Check is whether the struct has a method check() error for the rules
	// across its fields.
	Check bool
	// Description is what a JSON Schema says about the struct: the lines of its
	// //demi:describe directives, joined by newlines.
	Description string
	// Schema is whether the package emits the struct's JSON Schema.
	Schema bool
	// Open is whether the struct ignores members it does not have, as a serde
	// struct without deny_unknown_fields does, where the others refuse them.
	Open bool
	// Scalar is the basic type of a variant that is not an object but a JSON
	// string, number or boolean; such a variant has no fields.
	Scalar *Type
	// Embeds are the structs whose members the struct's Fields include, in
	// the order it embeds them.
	Embeds []string
	// variant is whether the struct is marked //demi:variant.
	variant bool
}

// A Field is a member of a wire struct.
type Field struct {
	// Name is the Go name and JSON is the member's name on the wire.
	Name string
	JSON string
	Doc  string
	Type *Type
	// Required is whether the member must be present: it has no omitzero.
	Required bool
	// Nullable is whether a required member may be null; its type is a pointer.
	Nullable bool
	// Rules are the rules of the field's check tag, after those of its type.
	Rules []Rule
}

// A Kind says what a wire type holds.
type Kind int

// The kinds of type a wire struct's field can have.
const (
	KindString Kind = iota + 1
	KindBool
	KindInt
	KindUint
	KindFloat
	KindStruct
	KindUnion
	KindSlice
	KindMap
	KindPointer
	// KindRaw is a jsontext.Value: JSON that the contract declares opaque.
	KindRaw
)

// A Type is the type of a field, as far as the wire cares.
type Type struct {
	Kind Kind
	// Src is the type as Go source.
	Src string
	// Name is the name of a named type: a struct, a union, or a type named
	// after a basic type; empty for an unnamed one.
	Name string
	// Bits is the size of an integer type; 0 is the size of an int.
	Bits int
	// Elem is the element of a slice, a pointer or a map.
	Elem *Type
	// Key is the key of a map.
	Key *Type
	// Opaque is whether a struct is one the package decodes itself.
	Opaque bool
	// Description and Rules belong to a type named after a basic type: a
	// closed set of strings (//demi:enum) or a string with rules (//demi:value).
	// The rules of a type apply to every field of it.
	Description string
	Rules       []Rule
}

// A Union is a sealed interface, Go's sum type.
type Union struct {
	Name string
	Doc  string
	// TagName is the member that tells the variants apart; empty for a union
	// told apart by its members.
	TagName string
	// Sealed is the interface's unexported method.
	Sealed   string
	Variants []*Struct
	// Description and Schema are a struct's.
	Description string
	Schema      bool
}

// A RuleKind is the name of a rule in a check tag.
type RuleKind string

// The rules of a check tag.
const (
	RuleChars   RuleKind = "chars"
	RuleBytes   RuleKind = "bytes"
	RuleItems   RuleKind = "items"
	RuleRange   RuleKind = "range"
	RuleEq      RuleKind = "eq"
	RuleOneOf   RuleKind = "oneof"
	RulePattern RuleKind = "pattern"
	RuleNoNUL   RuleKind = "nonul"
	RuleUnique  RuleKind = "unique"
	RuleFunc    RuleKind = "func"
	RuleEach    RuleKind = "each"
	RuleKeys    RuleKind = "keys"
	// RuleNullable is not a check but a member's mark: a required member may be
	// null. The reader takes it out of a field's rules.
	RuleNullable RuleKind = "nullable"
)

// A Rule is one rule of a check tag.
type Rule struct {
	Kind RuleKind
	// Min and Max are the bounds of chars, bytes, items and range.
	Min, Max *Bound
	// Value is the operand of eq, pattern and func.
	Value string
	// Values are the strings of oneof.
	Values []string
	// Inner are the rules of each and keys.
	Inner []Rule
}

// A Bound is one end of a range: a number, or the name of a constant.
type Bound struct {
	// Src is the bound as Go source.
	Src string
	// Value is the bound's number; Float is the same number when the bound of
	// a float has a fraction.
	Value int64
	Float float64
	// Fraction is whether the bound has a fraction, and so Float is exact and
	// Value is not.
	Fraction bool
}

// jsonKind names the JSON kind that a value of a basic type is written as.
func (t *Type) jsonKind() string {
	switch t.Kind {
	case KindString:
		return "string"
	case KindBool:
		return "boolean"
	default:
		return "number"
	}
}

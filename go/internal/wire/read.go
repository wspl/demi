package wire

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"slices"
	"strconv"
)

// The rules of the structure of a value, worded once.
const (
	ruleRequired      = "required"
	ruleUnknownMember = "unknown member"
	ruleNull          = "must not be null"
	ruleNotJSON       = "is not valid JSON"
	ruleDuplicate     = "has a duplicate member"
	ruleNoFit         = "does not fit its type"
)

// IntBits is the size of an int.
const IntBits = strconv.IntSize

// syntaxRule words the failure of the JSON decoder itself.
func syntaxRule(err error) string {
	if errors.Is(err, jsontext.ErrDuplicateName) {
		return ruleDuplicate
	}
	return ruleNotJSON
}

// Required returns the refusal of a member that a value lacks.
func Required(name string) error {
	return &InvalidError{Path: name, Rule: ruleRequired}
}

// Unknown returns the refusal of a member that the type does not have.
func Unknown(name string) error {
	return &InvalidError{Path: name, Rule: ruleUnknownMember}
}

// Null returns the refusal of a null where a value is optional: an optional
// member is absent or holds a value.
func Null() error {
	return &InvalidError{Rule: ruleNull}
}

// IsNull reports whether the next value is a null.
func IsNull(dec *jsontext.Decoder) bool {
	return dec.PeekKind() == 'n'
}

func mustBe(what string) error {
	return &InvalidError{Rule: "must be " + what}
}

// BeginObject reads the start of an object.
func BeginObject(dec *jsontext.Decoder) error {
	token, err := dec.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '{' {
		return mustBe("an object")
	}
	return nil
}

// NextMember reads the name of the next member of an object, or its end.
func NextMember(dec *jsontext.Decoder) (name string, more bool, err error) {
	token, err := dec.ReadToken()
	if err != nil {
		return "", false, err
	}
	if token.Kind() == '}' {
		return "", false, nil
	}
	return token.String(), true, nil
}

// BeginArray reads the start of an array.
func BeginArray(dec *jsontext.Decoder) error {
	token, err := dec.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '[' {
		return mustBe("an array")
	}
	return nil
}

// NextElement reports whether an array has another element; at its end it
// reads the end.
func NextElement(dec *jsontext.Decoder) (bool, error) {
	if dec.PeekKind() == ']' {
		_, err := dec.ReadToken()
		return false, err
	}
	// A failed peek leaves the read that follows to report it.
	return true, nil
}

// ReadString reads a string.
func ReadString(dec *jsontext.Decoder) (string, error) {
	token, err := dec.ReadToken()
	if err != nil {
		return "", err
	}
	if token.Kind() != '"' {
		return "", mustBe("a string")
	}
	return token.String(), nil
}

// ReadBool reads a boolean.
func ReadBool(dec *jsontext.Decoder) (bool, error) {
	token, err := dec.ReadToken()
	if err != nil {
		return false, err
	}
	if kind := token.Kind(); kind != 't' && kind != 'f' {
		return false, mustBe("a boolean")
	}
	return token.Bool(), nil
}

// ReadUint reads an integer that fits an unsigned integer of the given size in
// bits.
func ReadUint(dec *jsontext.Decoder, bits int) (uint64, error) {
	token, err := dec.ReadToken()
	if err != nil {
		return 0, err
	}
	if token.Kind() != '0' {
		return 0, mustBe("a number")
	}
	n, err := token.Uint()
	if err != nil || bits < 64 && n >= 1<<bits {
		return 0, mustBe("an unsigned integer of " + strconv.Itoa(bits) + " bits")
	}
	return n, nil
}

// ReadInt reads an integer that fits a signed integer of the given size in bits.
func ReadInt(dec *jsontext.Decoder, bits int) (int64, error) {
	token, err := dec.ReadToken()
	if err != nil {
		return 0, err
	}
	if token.Kind() != '0' {
		return 0, mustBe("a number")
	}
	n, err := token.Int()
	if err != nil || bits < 64 && (n >= 1<<(bits-1) || n < -1<<(bits-1)) {
		return 0, mustBe("an integer of " + strconv.Itoa(bits) + " bits")
	}
	return n, nil
}

// ReadRaw reads a value of any kind and returns a copy of its JSON, for a field
// the contract declares opaque.
func ReadRaw(dec *jsontext.Decoder) (jsontext.Value, error) {
	value, err := dec.ReadValue()
	if err != nil {
		return nil, err
	}
	return slices.Clone(value), nil
}

// ReadObject reads an object as its JSON, for a union that looks at the object
// before it knows what the object is.
func ReadObject(dec *jsontext.Decoder) (jsontext.Value, error) {
	value, err := dec.ReadValue()
	if err != nil {
		return nil, err
	}
	if value.Kind() != '{' {
		return nil, mustBe("an object")
	}
	return slices.Clone(value), nil
}

// NewDecoder returns a decoder of the JSON that ReadObject returned.
func NewDecoder(value jsontext.Value) *jsontext.Decoder {
	return jsontext.NewDecoder(bytes.NewReader(value))
}

// Tag returns the string member name of the object value, which tells what the
// object is.
func Tag(value jsontext.Value, name string) (string, error) {
	dec := NewDecoder(value)
	if _, err := dec.ReadToken(); err != nil {
		return "", err
	}
	for {
		member, more, err := NextMember(dec)
		if err != nil {
			return "", err
		}
		if !more {
			return "", Required(name)
		}
		if member != name {
			if err := dec.SkipValue(); err != nil {
				return "", err
			}
			continue
		}
		tag, err := ReadString(dec)
		if err != nil {
			return "", In(name, err)
		}
		return tag, nil
	}
}

// UnknownTag returns the refusal of a tag that names none of a union's
// variants.
func UnknownTag(name string, tags ...string) error {
	return &InvalidError{Path: name, Rule: "must be one of: " + join(tags)}
}

// NoVariant returns the refusal of an object that fits none of the variants of
// a union told apart by its members.
func NoVariant(variants ...string) error {
	return &InvalidError{Rule: "matches none of: " + join(variants)}
}

// SeveralVariants returns the refusal of an object that fits more than one
// variant of a union told apart by its members.
func SeveralVariants(variants ...string) error {
	return &InvalidError{Rule: "matches more than one of: " + join(variants)}
}

func join(names []string) string {
	var joined bytes.Buffer
	for i, name := range names {
		if i > 0 {
			joined.WriteString(", ")
		}
		joined.WriteString(name)
	}
	return joined.String()
}

// Refusal returns err, the failure of decoding a document as JSON with the
// generated decoders or of encoding a value, as the *InvalidError that names
// its field. A failure that no generated method has named (a document that ends
// early, text after its value, a string that is not UTF-8) is named by the JSON
// pointer that the JSON package reports, written as a path.
func Refusal(err error) error {
	var invalid *InvalidError
	if errors.As(err, &invalid) {
		return invalid
	}
	var syntactic *jsontext.SyntacticError
	if errors.As(err, &syntactic) {
		return &InvalidError{Path: pathOf(syntactic.JSONPointer), Rule: syntaxRule(err)}
	}
	var semantic *json.SemanticError
	if errors.As(err, &semantic) {
		return &InvalidError{Path: pathOf(semantic.JSONPointer), Rule: ruleNoFit}
	}
	return &InvalidError{Rule: ruleNotJSON}
}

// pathOf writes a JSON pointer as a path: a token of digits is an index.
func pathOf(pointer jsontext.Pointer) string {
	path := ""
	for token := range pointer.Tokens() {
		elem := token
		if _, err := strconv.ParseUint(token, 10, 64); err == nil {
			elem = "[" + token + "]"
		}
		path = Prefix(path, elem)
	}
	return path
}

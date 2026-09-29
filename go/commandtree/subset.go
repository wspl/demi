package commandtree

import (
	"slices"
)

// The keywords an input object may say besides its fields.
var objectKeywords = []string{"type", "properties", "required", "additionalProperties", "title", "description"}

// The keywords a field's schema may say.
var fieldKeywords = []string{
	"type", "enum", "items", "description", "format", "minLength", "maxLength", "pattern",
	"minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minItems", "maxItems",
}

// CheckInputSubset checks that schema is inside the command input subset
// (docs/execution/commands.md, The input subset): an object that allows no other
// properties, whose fields are strings, numbers, integers, booleans, string
// enums or arrays of one of those, each with only the bounds the subset names.
// A refusal names the field and why, as a [*DeclarationError]. Registration runs
// it; argv parsing and help read each field's schema themselves.
func CheckInputSubset(schema *Schema) error {
	object := schema.document
	// The schema is a map in the Rust implementation, so its keywords are
	// checked in the order of their names.
	for _, keyword := range object.sortedNames() {
		if !slices.Contains(objectKeywords, keyword) {
			return declarationf("command input uses \"%s\", outside the command input subset", keyword)
		}
	}
	if typeOf(object) != "object" {
		return declarationf("command input must describe an object")
	}
	if additional, _ := object.Get("additionalProperties"); additional != false {
		return declarationf("command input must refuse unknown fields (additionalProperties: false)")
	}
	var properties Object
	if member, ok := object.Get("properties"); ok {
		var isObject bool
		properties, isObject = member.(Object)
		if !isObject {
			return declarationf("command input properties must be an object")
		}
	}
	if member, ok := object.Get("required"); ok {
		names, isList := member.([]any)
		if !isList {
			return declarationf("command input required must be a list")
		}
		for _, name := range names {
			declared := false
			if text, ok := name.(string); ok {
				_, declared = properties.Get(text)
			}
			if !declared {
				return declarationf("command input requires an undeclared field %s", encodeValue(name))
			}
		}
	}
	for name, field := range properties.All() {
		if reason := refuseField(field); reason != "" {
			return declarationf("input \"%s\": %s", name, reason)
		}
	}
	return nil
}

// refuseField returns why the subset refuses a field's schema, or "" when it
// takes it.
func refuseField(field any) string {
	schema, ok := field.(Object)
	if !ok {
		return "must be a schema object"
	}
	if _, ok := schema.Get("default"); ok {
		return "carries a default; a missing value is the handler's to supply"
	}
	for _, keyword := range []string{"oneOf", "anyOf", "allOf"} {
		if _, ok := schema.Get(keyword); ok {
			return "is a union, which has no command-line form"
		}
	}
	if _, ok := schema.Get("$ref"); ok {
		return "refers to another schema"
	}
	if _, ok := schema.Get("properties"); ok {
		return "is a nested object, which has no command-line form"
	}
	for keyword := range schema.All() {
		if !slices.Contains(fieldKeywords, keyword) {
			return "uses " + quote(keyword) + ", outside the command input subset"
		}
	}
	var kind string
	switch declared, _ := schema.Get("type"); declared := declared.(type) {
	case string:
		kind = declared
	case []any:
		if slices.Contains(declared, any("null")) {
			return "allows null; an optional field is absent instead"
		}
		return "has several types"
	default:
		return "declares no type"
	}
	_, hasFormat := schema.Get("format")
	if hasFormat && kind != "integer" && kind != "number" {
		return "carries a format, which only a number's schema may"
	}
	if _, ok := schema.Get("items"); ok && kind != "array" {
		return "has items but is not an array"
	}
	_, hasEnum := schema.Get("enum")
	switch {
	case kind == "string":
		return refuseEnum(schema)
	case hasEnum:
		return "is an enum of values that are not strings"
	case kind == "number" || kind == "integer" || kind == "boolean":
		return ""
	case kind == "array":
		items, ok := schema.Get("items")
		if !ok {
			return "is an array without items"
		}
		if typeOf(items) == "array" {
			return "is an array of arrays, which has no command-line form"
		}
		if reason := refuseField(items); reason != "" {
			return "its items: " + reason
		}
		return ""
	case kind == "object":
		return "is a nested object, which has no command-line form"
	}
	return "has type " + quote(kind) + ", outside the command input subset"
}

// refuseEnum returns why the subset refuses the enum of a string field.
func refuseEnum(schema Object) string {
	member, ok := schema.Get("enum")
	if !ok {
		return ""
	}
	values, isList := member.([]any)
	if !isList {
		return "has an enum that is not a list"
	}
	for _, value := range values {
		switch value.(type) {
		case string:
		case nil:
			return "allows null; an optional field is absent instead"
		default:
			return "is an enum of values that are not all strings"
		}
	}
	return ""
}

// quote wraps a keyword or a type in the double quotes the messages use.
func quote(text string) string {
	return `"` + text + `"`
}

package declare

import (
	"errors"
	"fmt"
	"maps"
	"slices"
)

// CheckInputSubset checks the schema forms registration permits as command input.
// It does not apply to output schemas, which may use the full JSON Schema language.
func CheckInputSubset(schema *Schema) error {
	if schema == nil || schema.validator == nil {
		return errors.New("uninitialized schema")
	}
	object := schema.object
	allowed := []string{"type", "properties", "required", "additionalProperties", "title", "description"}
	for _, keyword := range slices.Sorted(maps.Keys(object)) {
		if !slices.Contains(allowed, keyword) {
			return fmt.Errorf("command input uses %q, outside the command input subset", keyword)
		}
	}
	if object["type"] != "object" {
		return errors.New("command input must describe an object")
	}
	if object["additionalProperties"] != false {
		return errors.New("command input must refuse unknown fields (additionalProperties: false)")
	}
	properties, ok := object["properties"].(map[string]any)
	if _, present := object["properties"]; present && !ok {
		return errors.New("command input properties must be an object")
	}
	if required, present := object["required"]; present {
		names, ok := required.([]any)
		if !ok {
			return errors.New("command input required must be a list")
		}
		for _, name := range names {
			text, ok := name.(string)
			if _, present := properties[text]; !ok || !present {
				return fmt.Errorf("command input requires an undeclared field %q", name)
			}
		}
	}
	for _, name := range slices.Sorted(maps.Keys(properties)) {
		if err := checkField(properties[name]); err != nil {
			return fmt.Errorf("input %q: %w", name, err)
		}
	}
	return nil
}

// checkField checks whether an input field has an unambiguous argv form.
func checkField(value any) error {
	schema, ok := value.(map[string]any)
	if !ok {
		return errors.New("must be a schema object")
	}
	if _, present := schema["default"]; present {
		return errors.New("carries a default; a missing value is the handler's to supply")
	}
	for _, keyword := range []string{"oneOf", "anyOf", "allOf"} {
		if _, present := schema[keyword]; present {
			return errors.New("is a union, which has no command-line form")
		}
	}
	if _, present := schema["$ref"]; present {
		return errors.New("refers to another schema")
	}
	if _, present := schema["properties"]; present {
		return errors.New("is a nested object, which has no command-line form")
	}
	allowed := []string{"type", "enum", "items", "description", "format", "minLength", "maxLength", "pattern", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minItems", "maxItems"}
	for _, keyword := range slices.Sorted(maps.Keys(schema)) {
		if !slices.Contains(allowed, keyword) {
			return fmt.Errorf("uses %q, outside the command input subset", keyword)
		}
	}
	kind, ok := schema["type"].(string)
	if !ok {
		if kinds, ok := schema["type"].([]any); ok {
			for _, kind := range kinds {
				if kind == "null" {
					return errors.New("allows null; an optional field is absent instead")
				}
			}
			return errors.New("has several types")
		}
		return errors.New("declares no type")
	}
	if _, present := schema["format"]; present && kind != "integer" && kind != "number" {
		return errors.New("carries a format, which only a number's schema may")
	}
	if _, present := schema["items"]; present && kind != "array" {
		return errors.New("has items but is not an array")
	}
	enum, hasEnum := schema["enum"]
	if kind == "string" {
		if !hasEnum {
			return nil
		}
		values, ok := enum.([]any)
		if !ok {
			return errors.New("has an enum that is not a list")
		}
		for _, value := range values {
			if value == nil {
				return errors.New("allows null; an optional field is absent instead")
			}
			if _, ok := value.(string); !ok {
				return errors.New("is an enum of values that are not all strings")
			}
		}
		return nil
	}
	if hasEnum {
		return errors.New("is an enum of values that are not strings")
	}
	switch kind {
	case "number", "integer", "boolean":
		return nil
	case "array":
		items, present := schema["items"]
		if !present {
			return errors.New("is an array without items")
		}
		if object, ok := items.(map[string]any); ok && object["type"] == "array" {
			return errors.New("is an array of arrays, which has no command-line form")
		}
		if err := checkField(items); err != nil {
			return fmt.Errorf("its items: %w", err)
		}
		return nil
	case "object":
		return errors.New("is a nested object, which has no command-line form")
	default:
		return fmt.Errorf("has type %q, outside the command input subset", kind)
	}
}

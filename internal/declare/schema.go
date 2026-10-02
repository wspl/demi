package declare

import (
	"bytes"
	"cmp"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
	"github.com/wspl/demi/internal/contract"
)

// Schema is an immutable schema document and its compiled validator.
// Construct it with NewSchema; copies share the read-only validator.
type Schema struct {
	document  json.RawMessage
	object    map[string]any
	validator *jsonschema.Schema
	order     map[string][]string
}

// NewSchema compiles a declaration's schema without loading external resources.
func NewSchema(document json.RawMessage) (_ *Schema, err error) {
	defer func() {
		if err != nil {
			err = &DeclarationError{err: err}
		}
	}()
	if err := contract.CheckJSON(document); err != nil {
		return nil, fmt.Errorf("schema: %w", err)
	}
	value, err := jsonschema.UnmarshalJSON(bytes.NewReader(document))
	if err != nil {
		return nil, fmt.Errorf("schema: %w", err)
	}
	object, ok := value.(map[string]any)
	if !ok {
		return nil, errors.New("schema must be an object")
	}
	compiler := jsonschema.NewCompiler()
	compiler.DefaultDraft(jsonschema.Draft2020)
	compiler.AssertContent()
	// An empty loader refuses filesystem and network access, including remote refs.
	compiler.UseLoader(jsonschema.SchemeURLLoader{})
	const location = "urn:demi:command-schema"
	if err := compiler.AddResource(location, object); err != nil {
		return nil, fmt.Errorf("schema: %w", err)
	}
	validator, err := compiler.Compile(location)
	if err != nil {
		return nil, fmt.Errorf("schema: %w", err)
	}
	order, err := documentOrder(document)
	if err != nil {
		return nil, fmt.Errorf("schema order: %w", err)
	}
	exhaustiveSchemas(validator)
	return &Schema{document: bytes.Clone(document), object: object, validator: validator, order: order}, nil
}

// Document returns a copy of the schema as declared, retaining property order.
func (s *Schema) Document() json.RawMessage { return bytes.Clone(s.document) }

// Check validates a JSON document without conversion or insertion of defaults.
// Failures name fields rather than disclose argument or output values.
func (s *Schema) Check(document json.RawMessage) error {
	if s == nil || s.validator == nil {
		return errors.New("uninitialized schema")
	}
	if err := contract.CheckJSON(document); err != nil {
		return fmt.Errorf("invalid JSON: %w", err)
	}
	value, err := jsonschema.UnmarshalJSON(bytes.NewReader(document))
	if err != nil {
		return fmt.Errorf("invalid JSON: %w", err)
	}
	if err := s.validator.Validate(value); err != nil {
		var validation *jsonschema.ValidationError
		if !errors.As(err, &validation) {
			return fmt.Errorf("schema validation: %w", err)
		}
		order, orderErr := documentOrder(document)
		if orderErr != nil {
			return fmt.Errorf("instance order: %w", orderErr)
		}
		failures := s.orderedFailures(flattenFailures(validation), order)
		var messages []string
		for _, failure := range failures {
			messages = append(messages, s.schemaFailures(failure, order)...)
		}
		if len(messages) == 0 {
			return nil
		}
		return errors.New(strings.Join(messages, "; "))
	}
	return nil
}

// schemaFailures renders the validator's decisions using command diagnostics.
func (s *Schema) schemaFailures(failure *jsonschema.ValidationError, order map[string][]string) []string {
	name := "value"
	if len(failure.InstanceLocation) != 0 {
		name = "\"" + strings.Join(failure.InstanceLocation, ".") + "\""
	}
	var message string
	switch reason := failure.ErrorKind.(type) {
	case *kind.Schema, *kind.Group, *kind.Reference, *kind.AllOf:
		children := s.orderedFailures(flattenChildren(failure), order)
		var messages []string
		for _, child := range children {
			messages = append(messages, s.schemaFailures(child, order)...)
		}
		return messages
	case *kind.Required:
		return requiredFailures(reason.Missing)
	case *kind.Type:
		if len(reason.Want) == 1 {
			message = fmt.Sprintf("%s is not of type %q", name, reason.Want[0])
		} else {
			types := slices.Clone(reason.Want)
			order := []string{"null", "boolean", "integer", "number", "string", "array", "object"}
			slices.SortFunc(types, func(a, b string) int { return cmp.Compare(slices.Index(order, a), slices.Index(order, b)) })
			quoted := make([]string, len(types))
			for i, value := range types {
				quoted[i] = fmt.Sprintf("%q", value)
			}
			message = name + " is not of types " + strings.Join(quoted, ", ")
		}
	case *kind.MinLength:
		message = fmt.Sprintf("%s is shorter than %d character%s", name, reason.Want, plural(reason.Want))
	case *kind.MaxLength:
		message = fmt.Sprintf("%s is longer than %d character%s", name, reason.Want, plural(reason.Want))
	case *kind.MinItems:
		message = fmt.Sprintf("%s has less than %d item%s", name, reason.Want, plural(reason.Want))
	case *kind.MaxItems:
		message = fmt.Sprintf("%s has more than %d item%s", name, reason.Want, plural(reason.Want))
	case *kind.Minimum:
		keyword := "minimum"
		message = name + " is less than the minimum of " + s.literal(append(schemaPath(failure.SchemaURL), keyword))
	case *kind.Maximum:
		keyword := "maximum"
		message = name + " is greater than the maximum of " + s.literal(append(schemaPath(failure.SchemaURL), keyword))
	case *kind.ExclusiveMinimum:
		keyword := "exclusiveMinimum"
		if value := s.schemaAt(append(schemaPath(failure.SchemaURL), keyword)); value == true {
			keyword = "minimum"
		}
		message = name + " is less than or equal to the minimum of " + s.literal(append(schemaPath(failure.SchemaURL), keyword))
	case *kind.ExclusiveMaximum:
		keyword := "exclusiveMaximum"
		if value := s.schemaAt(append(schemaPath(failure.SchemaURL), keyword)); value == true {
			keyword = "maximum"
		}
		message = name + " is greater than or equal to the maximum of " + s.literal(append(schemaPath(failure.SchemaURL), keyword))
	case *kind.Pattern:
		message = fmt.Sprintf("%s does not match \"%s\"", name, reason.Want)
	case *kind.Enum:
		options := make([]string, 0, len(reason.Want))
		for i := range reason.Want {
			options = append(options, s.literal(append(schemaPath(failure.SchemaURL), "enum", strconv.Itoa(i))))
		}
		if len(options) > 3 {
			options = append(options[:2], fmt.Sprintf("%d other candidates", len(options)-2))
		}
		message = name + " is not one of "
		if len(options) > 1 {
			message += strings.Join(options[:len(options)-1], ", ") + " or "
		}
		if len(options) > 0 {
			message += options[len(options)-1]
		}
	case *kind.AdditionalProperties:
		parent, _ := s.schemaAt(schemaPath(failure.SchemaURL)).(map[string]any)
		if parent["properties"] == nil && parent["patternProperties"] == nil && parent["additionalProperties"] == false {
			return []string{"False schema does not allow " + name}
		}
		message = unexpectedProperties("Additional", reason.Properties, failure.InstanceLocation, order)
	case *kind.Const:
		message = s.literal(append(schemaPath(failure.SchemaURL), "const")) + " was expected"
	case *kind.AnyOf:
		message = name + " is not valid under any of the schemas listed in the 'anyOf' keyword"
	case *kind.OneOf:
		if len(reason.Subschemas) == 0 {
			message = name + " is not valid under any of the schemas listed in the 'oneOf' keyword"
		} else {
			message = name + " is valid under more than one of the schemas listed in the 'oneOf' keyword"
		}
	case *kind.FalseSchema:
		message = "False schema does not allow " + name
	case *kind.UniqueItems:
		message = name + " has non-unique elements"
	case *kind.Format:
		message = fmt.Sprintf("%s is not a %q", name, reason.Want)
	case *kind.MinProperties:
		suffix := "ies"
		if reason.Want == 1 {
			suffix = "y"
		}
		message = fmt.Sprintf("%s has less than %d propert%s", name, reason.Want, suffix)
	case *kind.MaxProperties:
		suffix := "ies"
		if reason.Want == 1 {
			suffix = "y"
		}
		message = fmt.Sprintf("%s has more than %d propert%s", name, reason.Want, suffix)
	case *kind.MultipleOf:
		keyword := "multipleOf"
		message = name + " is not a multiple of " + s.literal(append(schemaPath(failure.SchemaURL), keyword))
	case *kind.Contains, *kind.MinContains, *kind.MaxContains:
		message = "None of " + name + " are valid under the given schema"
	case *kind.Not:
		message = s.literal(append(schemaPath(failure.SchemaURL), "not")) + " is not allowed for " + name
	case *kind.AdditionalItems:
		value := s.schemaAt(append(schemaPath(failure.SchemaURL), "items"))
		items, _ := value.([]any)
		message = fmt.Sprintf("Additional items are not allowed (%d items)", len(items))
	case *kind.Dependency:
		return requiredFailures(reason.Missing)
	case *kind.DependentRequired:
		return requiredFailures(reason.Missing)
	case *kind.ContentEncoding:
		message = fmt.Sprintf("%s is not compliant with %q content encoding", name, reason.Want)
	case *kind.ContentMediaType:
		if !utf8.Valid(reason.Got) {
			return []string{contentUTF8Error(reason.Got)}
		}
		message = fmt.Sprintf("%s is not compliant with %q media type", name, reason.Want)
	case *kind.ContentSchema:
		var messages []string
		for _, child := range failure.Causes {
			messages = append(messages, s.schemaFailures(child, order)...)
		}
		return messages
	case *kind.PropertyNames:
		var messages []string
		children := s.orderedFailures(flattenChildren(failure), order)
		for _, child := range children {
			rendered := s.schemaFailures(child, order)
			for _, text := range rendered {
				messages = append(messages, strings.Replace(text, "value", strconv.Quote(reason.Property), 1))
			}
		}
		return messages
	case *unevaluatedFailure:
		if reason.items {
			message = fmt.Sprintf("Unevaluated items are not allowed (%d items)", len(reason.names))
		} else {
			message = unexpectedProperties("Unevaluated", reason.names, failure.InstanceLocation, order)
		}
	case *kind.RefCycle:
		// The Rust compiler elides a direct recursive reference at the same instance.
		return nil
	case *kind.InvalidJsonValue:
		// Check decodes JSON before invoking the validator, so this is unreachable.
		return nil
	}
	return []string{message}
}

// plural supplies the suffix used by command-schema length diagnostics.
func plural(count int) string {
	if count == 1 {
		return ""
	}
	return "s"
}

// flattenFailures removes library grouping nodes while retaining keyword failures.
func flattenFailures(failure *jsonschema.ValidationError) []*jsonschema.ValidationError {
	switch failure.ErrorKind.(type) {
	case *kind.Schema, *kind.Group, *kind.AllOf:
		return flattenChildren(failure)
	}
	return []*jsonschema.ValidationError{failure}
}

// flattenChildren retains reference invocation boundaries while unwrapping error groups.
func flattenChildren(failure *jsonschema.ValidationError) []*jsonschema.ValidationError {
	var children []*jsonschema.ValidationError
	for _, child := range failure.Causes {
		children = append(children, flattenFailures(child)...)
	}
	return children
}

// contentUTF8Error preserves Rust's decoded-content UTF-8 diagnostic.
func contentUTF8Error(data []byte) string {
	for index := 0; index < len(data); {
		r, size := utf8.DecodeRune(data[index:])
		if r != utf8.RuneError || size != 1 {
			index += size
			continue
		}
		first := data[index]
		width := 1
		switch {
		case first >= 0xc2 && first <= 0xdf:
			width = 2
		case first >= 0xe0 && first <= 0xef:
			width = 3
		case first >= 0xf0 && first <= 0xf4:
			width = 4
		}
		for offset := 1; offset < width; offset++ {
			if index+offset >= len(data) {
				return fmt.Sprintf("incomplete utf-8 byte sequence from index %d", index)
			}
			next := data[index+offset]
			if next < 0x80 || next > 0xbf || offset == 1 && (first == 0xe0 && next < 0xa0 || first == 0xed && next >= 0xa0 || first == 0xf0 && next < 0x90 || first == 0xf4 && next >= 0x90) {
				return fmt.Sprintf("invalid utf-8 sequence of %d bytes from index %d", offset, index)
			}
		}
		return fmt.Sprintf("invalid utf-8 sequence of 1 bytes from index %d", index)
	}
	return ""
}

// orderedFailures applies Rust's aggregation and diagnostic traversal order.
func (s *Schema) orderedFailures(failures []*jsonschema.ValidationError, order map[string][]string) []*jsonschema.ValidationError {
	failures = aggregateFailures(failures)
	slices.SortStableFunc(failures, func(a, b *jsonschema.ValidationError) int {
		return cmp.Compare(s.diagnosticRank(a, order), s.diagnosticRank(b, order))
	})
	return failures
}

// requiredFailures renders the required-field wording shared by dependency checks.
func requiredFailures(fields []string) []string {
	messages := make([]string, 0, len(fields))
	for _, field := range fields {
		messages = append(messages, fmt.Sprintf("%q is a required property", field))
	}
	return messages
}

// unexpectedProperties renders additional and unevaluated property refusals in input order.
func unexpectedProperties(keyword string, fields, location []string, order map[string][]string) string {
	names := slices.Clone(fields)
	prefix := ""
	for _, part := range location {
		prefix += "/" + pointerEscape(part)
	}
	slices.SortStableFunc(names, func(a, b string) int {
		return cmp.Compare(slices.Index(order[prefix], a), slices.Index(order[prefix], b))
	})
	for i, name := range names {
		names[i] = "'" + name + "'"
	}
	suffix := " were unexpected)"
	if len(names) == 1 {
		suffix = " was unexpected)"
	}
	return keyword + " properties are not allowed (" + strings.Join(names, ", ") + suffix
}

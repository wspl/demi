package declare

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"

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
}

// NewSchema compiles a declaration's schema without loading external resources.
func NewSchema(document json.RawMessage) (*Schema, error) {
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
	return &Schema{document: bytes.Clone(document), object: object, validator: validator}, nil
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
		return errors.New(strings.Join(schemaFailures(validation), "; "))
	}
	return nil
}

// schemaFailures renders the validator's decisions using command diagnostics.
func schemaFailures(failure *jsonschema.ValidationError) []string {
	name := "value"
	if len(failure.InstanceLocation) != 0 {
		name = fmt.Sprintf("%q", strings.Join(failure.InstanceLocation, "."))
	}
	var message string
	switch reason := failure.ErrorKind.(type) {
	case *kind.Schema, *kind.Group, *kind.Reference, *kind.AllOf:
		var messages []string
		for _, child := range failure.Causes {
			messages = append(messages, schemaFailures(child)...)
		}
		return messages
	case *kind.Required:
		messages := make([]string, 0, len(reason.Missing))
		for _, field := range reason.Missing {
			messages = append(messages, fmt.Sprintf("%q is a required property", field))
		}
		return messages
	case *kind.Type:
		if len(reason.Want) == 1 {
			message = fmt.Sprintf("%s is not of type %q", name, reason.Want[0])
		} else {
			quoted := make([]string, len(reason.Want))
			for i, value := range reason.Want {
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
		limit, _ := reason.Want.Float64()
		message = fmt.Sprintf("%s is less than the minimum of %v", name, limit)
	case *kind.Maximum:
		limit, _ := reason.Want.Float64()
		message = fmt.Sprintf("%s is greater than the maximum of %v", name, limit)
	case *kind.ExclusiveMinimum:
		limit, _ := reason.Want.Float64()
		message = fmt.Sprintf("%s is less than or equal to the minimum of %v", name, limit)
	case *kind.ExclusiveMaximum:
		limit, _ := reason.Want.Float64()
		message = fmt.Sprintf("%s is greater than or equal to the maximum of %v", name, limit)
	case *kind.Pattern:
		message = fmt.Sprintf("%s does not match \"%s\"", name, reason.Want)
	case *kind.Enum:
		options := make([]string, 0, len(reason.Want))
		for _, value := range reason.Want {
			encoded, err := json.Marshal(value)
			if err != nil {
				return []string{name + " does not match its enum"}
			}
			options = append(options, string(encoded))
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
		names := slices.Clone(reason.Properties)
		slices.Sort(names)
		for i, field := range names {
			names[i] = "'" + field + "'"
		}
		suffix := " were unexpected)"
		if len(names) == 1 {
			suffix = " was unexpected)"
		}
		message = "Additional properties are not allowed (" + strings.Join(names, ", ") + suffix
	case *kind.Const:
		encoded, err := json.Marshal(reason.Want)
		if err != nil {
			return []string{name + " does not match its constant"}
		}
		message = string(encoded) + " was expected"
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
	default:
		// Never use the library's default diagnostic: it can contain the input body.
		message = fmt.Sprintf("%s fails schema keyword %q", name, strings.Join(reason.KeywordPath(), "."))
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

package declare

import (
	"encoding/json"
	"fmt"
	"maps"
	"math"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// UsageError describes an invocation that does not fit its selected command.
type UsageError struct{ err error }

func (e *UsageError) Error() string { return e.err.Error() }
func (e *UsageError) Unwrap() error { return e.err }

// Selected records the command reached by argv and where its arguments start.
type Selected[B any] struct {
	Node          Node[B]
	Path          []string
	argumentIndex int
}

// Parsed holds argv input before or after validation.
type Parsed struct {
	Path       []string       `json:"path"`
	Values     map[string]any `json:"values"`
	JSON       bool           `json:"json"`
	Help       bool           `json:"help"`
	valueOrder []string
}

// Select finds the command named by argv, excluding the root executable name.
func (g *Group[B]) Select(argv []string) (*Selected[B], error) { return selectNode[B](g, argv) }

// Select selects this root leaf, whose arguments begin immediately.
func (l *Leaf[B]) Select(argv []string) (*Selected[B], error) { return selectNode[B](l, argv) }

// selectNode walks only group tokens, leaving leaf tokens for parsing.
func selectNode[B any](node Node[B], argv []string) (*Selected[B], error) {
	path := []string{Name(node)}
	index := 0
	for {
		group, ok := node.(*Group[B])
		if !ok || index == len(argv) || argv[index] == "--help" {
			break
		}
		var next Node[B]
		for _, child := range group.Subcommands {
			if Name(child) == argv[index] {
				next = child
				break
			}
		}
		if next == nil {
			return nil, usageError("Unknown subcommand \"%s %s\"", strings.Join(path, " "), argv[index])
		}
		node = next
		path = append(path, Name(node))
		index++
	}
	return &Selected[B]{Node: node, Path: path, argumentIndex: index}, nil
}

// Parse reads argv without reading stdin; help never consumes a body.
func (s *Selected[B]) Parse(argv []string) (*Parsed, error) {
	result := &Parsed{Path: slices.Clone(s.Path), Values: map[string]any{}}
	leaf := AsLeaf(s.Node)
	if leaf == nil {
		result.Help = true
		return result, nil
	}
	properties := leaf.properties()
	command := strings.Join(s.Path, " ")
	positional := 0
	optionsEnded := false
	for index := s.argumentIndex; index < len(argv); {
		token := argv[index]
		index++
		if !optionsEnded && token == "--" {
			if leaf.RestField != nil {
				rest := make([]any, len(argv)-index)
				for i, token := range argv[index:] {
					rest[i] = token
				}
				result.Values[*leaf.RestField] = rest
				result.valueOrder = append(result.valueOrder, *leaf.RestField)
				break
			}
			optionsEnded = true
			continue
		}
		if !optionsEnded && token == "--help" {
			result.Help = true
			return result, nil
		}
		if !optionsEnded && token == "--json" {
			if leaf.JSONOutput() == nil {
				return nil, usageError("Command \"%s\" does not define JSON output", command)
			}
			result.JSON = true
			continue
		}
		if !optionsEnded && strings.HasPrefix(token, "--") {
			field, inline, hasInline := strings.Cut(token[2:], "=")
			schema, exists := properties[field]
			if !exists {
				return nil, usageError("Unknown option \"--%s\" for \"%s\"", field, command)
			}
			if leaf.StdinField != nil && *leaf.StdinField == field {
				return nil, usageError("\"%s\" reads %s only from stdin. Remove --%s and use a quoted heredoc, pipe, or input redirection.", command, field, field)
			}
			source := ""
			if leaf.RestField != nil && *leaf.RestField == field {
				source = "passed after --"
			} else if slices.Contains(leaf.positionals(), field) {
				source = "a positional argument"
			}
			if source != "" {
				return nil, usageError("\"%s\" is %s for \"%s\"; --%s is not an option", field, source, command, field)
			}
			var value any
			if hasInline {
				value = inline
			} else if schemaType(schema) == "boolean" && (index == len(argv) || strings.HasPrefix(argv[index], "--")) {
				value = true
			} else {
				if index == len(argv) || strings.HasPrefix(argv[index], "--") {
					return nil, usageError("Missing value for \"--%s\"", field)
				}
				value = argv[index]
				index++
			}
			if err := result.setValue(field, value, schema); err != nil {
				return nil, err
			}
			continue
		}
		if positional == len(leaf.positionals()) {
			return nil, usageError("Unexpected positional argument \"%s\"", token)
		}
		field := leaf.positionals()[positional]
		positional++
		if err := result.setValue(field, token, properties[field]); err != nil {
			return nil, err
		}
	}
	return result, nil
}

// Validate supplies stdin and validates a manifest leaf after parsing argv.
func (p *Parsed) Validate(leaf *Leaf[Binding], stdin *string) (*Parsed, error) {
	return validateParsed(p, leaf, stdin)
}

// validateParsed converts argv values under the selected command's field schemas.
func validateParsed[B any](parsed *Parsed, leaf *Leaf[B], stdin *string) (*Parsed, error) {
	if parsed.Help {
		return parsed, nil
	}
	if leaf.StdinField != nil {
		if stdin == nil {
			return nil, usageError("stdin field was not supplied by dispatcher")
		}
		if _, exists := parsed.Values[*leaf.StdinField]; !exists {
			parsed.valueOrder = append(parsed.valueOrder, *leaf.StdinField)
		}
		parsed.Values[*leaf.StdinField] = *stdin
	} else if stdin != nil {
		return nil, usageError("stdin body supplied to a leaf without stdinField")
	}
	for field, value := range parsed.Values {
		if schema, exists := leaf.properties()[field]; exists {
			parsed.Values[field] = argvValue(value, schema)
		}
	}
	// Preserve argv order. Values added through the public map have no insertion
	// order in Go, so their diagnostic order is lexical.
	order := slices.Clone(parsed.valueOrder)
	for _, field := range slices.Sorted(maps.Keys(parsed.Values)) {
		if !slices.Contains(order, field) {
			order = append(order, field)
		}
	}
	fields := make([]contract.Field, 0, len(parsed.Values))
	for _, field := range order {
		if value, exists := parsed.Values[field]; exists {
			fields = append(fields, contract.Field{Name: field, Value: value})
		}
	}
	document, err := contract.EncodeObject(fields)
	if err != nil {
		return nil, usageError("Invalid command arguments: %w", err)
	}
	if err := leaf.CheckArguments(document); err != nil {
		return nil, err
	}
	return parsed, nil
}

// CheckArguments validates RPC or CLI arguments without coercing JSON values.
func (l *Leaf[B]) CheckArguments(arguments json.RawMessage) error {
	if l.Input == nil {
		object, err := contract.Object(arguments)
		if err != nil {
			return usageError("Invalid command arguments: %w", err)
		}
		if len(object) == 0 {
			return nil
		}
		return usageError("Invalid command arguments: the command takes no arguments")
	}
	if err := l.Input.Check(arguments); err != nil {
		return usageError("Invalid command arguments: %w", err)
	}
	return nil
}

// schemaType reads the input field's declared scalar type.
func schemaType(schema any) string {
	object, _ := schema.(map[string]any)
	kind, _ := object["type"].(string)
	return kind
}

// setValue accumulates repeated array options and rejects duplicate scalars.
func (p *Parsed) setValue(field string, value, schema any) error {
	values := p.Values
	previous, exists := values[field]
	if !exists {
		p.valueOrder = append(p.valueOrder, field)
		values[field] = value
		return nil
	}
	if schemaType(schema) != "array" {
		return usageError("Duplicate value for \"%s\"", field)
	}
	if items, ok := previous.([]any); ok {
		values[field] = append(items, value)
	} else {
		values[field] = []any{previous, value}
	}
	return nil
}

// argvValue turns CLI text into the value the command field declares.
func argvValue(value, schema any) any {
	switch schemaType(schema) {
	case "array":
		object, _ := schema.(map[string]any)
		items, ok := value.([]any)
		if !ok {
			items = []any{value}
		}
		converted := make([]any, len(items))
		for i, item := range items {
			converted[i] = argvValue(item, object["items"])
		}
		return converted
	case "number", "integer":
		if text, ok := value.(string); ok {
			text = strings.TrimSpace(text)
			// Rust's float grammar accepts decimal floats, but not Go's hex floats or underscores.
			if strings.ContainsAny(text, "_xXpP") {
				return value
			}
			number, err := strconv.ParseFloat(text, 64)
			if err == nil && !math.IsNaN(number) && !math.IsInf(number, 0) {
				if math.Trunc(number) == number && math.Abs(number) <= 9007199254740991 {
					return int64(number)
				}
				return number
			}
		}
	case "boolean":
		if text, ok := value.(string); ok {
			if text == "true" {
				return true
			}
			if text == "false" {
				return false
			}
		}
	}
	return value
}

// usageError keeps the command protocol's capitalized, user-facing diagnostics
// and retains wrapped causes for errors.Is/errors.As.
func usageError(format string, args ...any) *UsageError {
	return &UsageError{err: fmt.Errorf(format, args...)}
}

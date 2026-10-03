package declare

import (
	"encoding/json"
	"fmt"
	"math"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// Selected records the command reached by argv and where its arguments start.
type Selected[B any] struct {
	// Node is the selected command or group.
	Node Node[B]
	// Path lists command names from the root.
	Path          []string
	argumentIndex int
}

// Parsed holds argv input before or after validation.
type Parsed struct {
	// Path lists command names from the root.
	Path []string `json:"path"`
	// Values holds command inputs in insertion order.
	Values Arguments `json:"values"`
	// JSON records a request for machine-readable output.
	JSON bool `json:"json"`
	// Help records a request for documentation.
	Help bool `json:"help"`
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
			//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
			return nil, fmt.Errorf("Unknown subcommand \"%s %s\"", strings.Join(path, " "), argv[index])
		}
		node = next
		path = append(path, Name(node))
		index++
	}
	return &Selected[B]{Node: node, Path: path, argumentIndex: index}, nil
}

// Parse reads argv without reading stdin; help never consumes a body.
func (s *Selected[B]) Parse(argv []string) (*Parsed, error) {
	result := &Parsed{Path: slices.Clone(s.Path)}
	leaf, ok := s.Node.(*Leaf[B])
	if !ok {
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
				result.setRest(*leaf.RestField, argv[index:])
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
				//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
				return nil, fmt.Errorf("Command \"%s\" does not define JSON output", command)
			}
			result.JSON = true
			continue
		}
		if !optionsEnded && strings.HasPrefix(token, "--") {
			consumed, err := leaf.parseOption(token, argv[index:], result, command)
			if err != nil {
				return nil, err
			}
			index += consumed
			continue
		}
		if positional == len(leaf.positionals()) {
			//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
			return nil, fmt.Errorf("Unexpected positional argument \"%s\"", token)
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
	return validate(p, leaf, stdin)
}

// validate converts argv values under the selected command's field schemas.
func validate[B any](parsed *Parsed, leaf *Leaf[B], stdin *string) (*Parsed, error) {
	if parsed.Help {
		return parsed, nil
	}
	if leaf.StdinField != nil {
		if stdin == nil {
			return nil, fmt.Errorf("stdin field was not supplied by dispatcher")
		}
		parsed.Values.Set(*leaf.StdinField, *stdin)
	} else if stdin != nil {
		return nil, fmt.Errorf("stdin body supplied to a leaf without stdinField")
	}
	for i, field := range parsed.Values.fields {
		if schema, exists := leaf.properties()[field.Name]; exists {
			parsed.Values.fields[i].Value = argvValue(field.Value, schema)
		}
	}
	document, err := parsed.Values.MarshalJSON()
	if err != nil {
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return nil, fmt.Errorf("Invalid command arguments: %w", err)
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
			//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
			return fmt.Errorf("Invalid command arguments: %w", err)
		}
		if len(object) == 0 {
			return nil
		}
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return fmt.Errorf("Invalid command arguments: the command takes no arguments")
	}
	if err := l.Input.Check(arguments); err != nil {
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return fmt.Errorf("Invalid command arguments: %w", err)
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
	previous, exists := p.Values.Lookup(field)
	if !exists {
		p.Values.Set(field, value)
		return nil
	}
	if schemaType(schema) != "array" {
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return fmt.Errorf("Duplicate value for \"%s\"", field)
	}
	if items, ok := previous.([]any); ok {
		p.Values.Set(field, append(items, value))
	} else {
		p.Values.Set(field, []any{previous, value})
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
			// Numbers are decimal: a hex float or a digit separator stays text, which the schema refuses.
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

// parseOption returns the number of following argv tokens consumed by one option.
func (l *Leaf[B]) parseOption(token string, argv []string, result *Parsed, command string) (int, error) {
	consumed := 0
	field, inline, hasInline := strings.Cut(token[2:], "=")
	schema, exists := l.properties()[field]
	if !exists {
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return 0, fmt.Errorf("Unknown option \"--%s\" for \"%s\"", field, command)
	}
	if l.StdinField != nil && *l.StdinField == field {
		//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
		return 0, fmt.Errorf(
			"\"%s\" reads %s only from stdin. Remove --%s and use a quoted heredoc, pipe, or input redirection.",
			command,
			field,
			field,
		)
	}
	source := ""
	if l.RestField != nil && *l.RestField == field {
		source = "passed after --"
	} else if slices.Contains(l.positionals(), field) {
		source = "a positional argument"
	}
	if source != "" {
		return 0, fmt.Errorf("\"%s\" is %s for \"%s\"; --%s is not an option", field, source, command, field)
	}
	var value any
	if hasInline {
		value = inline
	} else if schemaType(schema) == "boolean" && (len(argv) == 0 || strings.HasPrefix(argv[0], "--")) {
		value = true
	} else {
		if len(argv) == 0 || strings.HasPrefix(argv[0], "--") {
			//nolint:revive,staticcheck // error-strings, ST1005: product text, shown to the user as written.
			return 0, fmt.Errorf("Missing value for \"--%s\"", field)
		}
		value = argv[0]
		consumed++
	}
	if err := result.setValue(field, value, schema); err != nil {
		return 0, err
	}
	return consumed, nil
}

func (p *Parsed) setRest(field string, argv []string) {
	rest := make([]any, len(argv))
	for i, token := range argv {
		rest[i] = token
	}
	p.Values.Set(field, rest)
}

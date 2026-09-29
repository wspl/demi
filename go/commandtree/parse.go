package commandtree

import (
	"errors"
	"math"
	"regexp"
	"slices"
	"strconv"
	"strings"
)

// A Selected is the command that an argv names below a root, and where its
// arguments begin.
type Selected struct {
	Node Node
	// Path names the command from its root: the root's name, then each group.
	Path          []string
	argumentIndex int
}

// A Parsed is what a command line comes to: the command, the values it fills
// and whether it asked for JSON output or help.
type Parsed struct {
	Path   []string
	Values Object
	JSON   bool
	Help   bool
}

// Select returns the command that argv names below root. argv excludes the
// root executable's name.
func Select(root Node, argv []string) (Selected, error) {
	node := root
	path := []string{Name(root)}
	index := 0
	for {
		group, ok := node.(Group)
		if !ok || index >= len(argv) || argv[index] == "--help" {
			break
		}
		token := argv[index]
		found := false
		for _, child := range group.Subcommands {
			if Name(child) == token {
				node = child
				found = true
				break
			}
		}
		if !found {
			return Selected{}, usagef("Unknown subcommand \"%s\"", strings.Join(path, " ")+" "+token)
		}
		path = append(path, Name(node))
		index++
	}
	return Selected{Node: node, Path: path, argumentIndex: index}, nil
}

// Parse parses argv without reading stdin. Help therefore never consumes a
// body.
func (s Selected) Parse(argv []string) (Parsed, error) {
	result := Parsed{Path: slices.Clone(s.Path)}
	leaf, ok := s.Node.(Leaf)
	if !ok {
		result.Help = true
		return result, nil
	}
	properties, _ := leaf.Properties()
	// The command's name in refusals, since a script may run several.
	command := strings.Join(s.Path, " ")
	positionals := valueOf(leaf.Positionals)
	index := s.argumentIndex
	positional := 0
	optionsEnded := false
	for index < len(argv) {
		token := argv[index]
		index++
		if !optionsEnded && token == "--" {
			if leaf.RestField != nil {
				rest := make([]any, 0, len(argv)-index)
				for _, token := range argv[index:] {
					rest = append(rest, token)
				}
				result.Values.Set(*leaf.RestField, rest)
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
				return Parsed{}, usagef("Command \"%s\" does not define JSON output", command)
			}
			result.JSON = true
			continue
		}
		if option, isOption := strings.CutPrefix(token, "--"); isOption && !optionsEnded {
			field, inline, hasInline := strings.Cut(option, "=")
			schema, declared := properties.Get(field)
			if !declared {
				return Parsed{}, usagef("Unknown option \"--%s\" for \"%s\"", field, command)
			}
			if leaf.StdinField != nil && *leaf.StdinField == field {
				return Parsed{}, usagef("\"%s\" reads %s only from stdin. Remove --%s and use a quoted heredoc, pipe, or input redirection.", command, field, field)
			}
			switch sourceOf(leaf, field) {
			case sourceRest:
				return Parsed{}, usagef("\"%s\" is passed after -- for \"%s\"; --%s is not an option", field, command, field)
			case sourcePositional:
				return Parsed{}, usagef("\"%s\" is a positional argument for \"%s\"; --%s is not an option", field, command, field)
			}
			var value any
			switch {
			case hasInline:
				value = inline
			case typeOf(schema) == "boolean" && (index >= len(argv) || strings.HasPrefix(argv[index], "--")):
				value = true
			case index >= len(argv) || strings.HasPrefix(argv[index], "--"):
				return Parsed{}, usagef("Missing value for \"--%s\"", field)
			default:
				value = argv[index]
				index++
			}
			if err := setValue(&result.Values, field, value, schema); err != nil {
				return Parsed{}, err
			}
			continue
		}
		if positional >= len(positionals) {
			return Parsed{}, usagef("Unexpected positional argument \"%s\"", token)
		}
		field := positionals[positional]
		positional++
		schema, _ := properties.Get(field)
		if err := setValue(&result.Values, field, token, schema); err != nil {
			return Parsed{}, err
		}
	}
	return result, nil
}

// Validate adds the body read from stdin, turns argv text into the values the
// fields declare, and checks the whole input. A missing value stays missing:
// validation never fills one in. The result has its own values; p is left as it
// was.
func (p Parsed) Validate(leaf Leaf, stdin *string) (Parsed, error) {
	if p.Help {
		return p, nil
	}
	values := p.Values.clone()
	switch {
	case leaf.StdinField != nil && stdin == nil:
		return Parsed{}, usagef("stdin field was not supplied by dispatcher")
	case leaf.StdinField != nil:
		values.Set(*leaf.StdinField, *stdin)
	case stdin != nil:
		return Parsed{}, usagef("stdin body supplied to a leaf without stdinField")
	}
	if properties, ok := leaf.Properties(); ok {
		// Setting the value of a member that is there keeps the order of the
		// members, so it is safe while they are read.
		for field, value := range values.All() {
			if schema, declared := properties.Get(field); declared {
				values.Set(field, argvValue(value, schema))
			}
		}
	}
	if err := leaf.CheckArguments(values); err != nil {
		return Parsed{}, err
	}
	p.Values = values
	return p, nil
}

// CheckArguments checks a command's arguments against its input, as the runner
// does after parsing argv and the backend does with an rpc call's arguments:
// one refusal names every field that fails. It returns a [*UsageError].
func (l Leaf) CheckArguments(arguments Object) error {
	var err error
	switch {
	case l.Input != nil:
		err = l.Input.Check(arguments)
	case arguments.Len() > 0:
		err = errors.New("the command takes no arguments")
	}
	if err != nil {
		return usagef("Invalid command arguments: %s", err)
	}
	return nil
}

// A source is where a command line supplies a field.
type source int

const (
	sourceOption source = iota
	sourcePositional
	sourceStdin
	sourceRest
)

func sourceOf(leaf Leaf, field string) source {
	switch {
	case leaf.StdinField != nil && *leaf.StdinField == field:
		return sourceStdin
	case leaf.RestField != nil && *leaf.RestField == field:
		return sourceRest
	}
	for _, positional := range valueOf(leaf.Positionals) {
		if positional == field {
			return sourcePositional
		}
	}
	return sourceOption
}

// setValue records a value of a field: a repeated option of an array field
// collects its values, and any other repeat is refused.
func setValue(values *Object, field string, value any, schema any) error {
	previous, seen := values.Get(field)
	if !seen {
		values.Set(field, value)
		return nil
	}
	if typeOf(schema) != "array" {
		return usagef("Duplicate value for \"%s\"", field)
	}
	if list, ok := previous.([]any); ok {
		values.Set(field, append(list, value))
	} else {
		values.Set(field, []any{previous, value})
	}
	return nil
}

// argvValue returns the value that an argv token stands for under its field's
// schema, since argv carries only text: a number, a boolean, or an array whose
// elements each convert by the items' schema. A token that spells no such value
// stays text, so the check names its field with every other failure.
func argvValue(value any, schema any) any {
	switch typeOf(schema) {
	case "array":
		items := lookup(schema, []string{"items"})
		if list, ok := value.([]any); ok {
			converted := make([]any, len(list))
			for i, item := range list {
				converted[i] = argvValue(item, items)
			}
			return converted
		}
		return []any{argvValue(value, items)}
	case "number", "integer":
		if text, ok := value.(string); ok {
			if parsed, ok := number(text); ok {
				return parsed
			}
		}
	case "boolean":
		switch value {
		case "true":
			return true
		case "false":
			return false
		}
	}
	return value
}

// floatSyntax is the text that Rust's f64 parser reads as a finite number.
var floatSyntax = regexp.MustCompile(`^[+-]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$`)

// maxExactInteger is the largest integer that JavaScript holds exactly.
const maxExactInteger = 9007199254740991

// number returns the finite number that text spells: an integer when it is one
// JavaScript holds exactly, so an integer field receives 2 rather than 2.0.
func number(text string) (any, bool) {
	text = strings.TrimSpace(text)
	if !floatSyntax.MatchString(text) {
		return nil, false
	}
	value, err := strconv.ParseFloat(text, 64)
	if err != nil && !errors.Is(err, strconv.ErrRange) || math.IsInf(value, 0) {
		return nil, false
	}
	if value == math.Trunc(value) && math.Abs(value) <= maxExactInteger {
		return int64(value), true
	}
	return value, true
}

package commanddecl

import (
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// HelpDefaults is the opening paragraph of the model's command documentation.
const HelpDefaults = "Unless a command states otherwise: success prints raw text on stdout, " +
	"failure writes an error message to stderr and exits non-zero. " +
	"Pass --help at any level to print a command's documentation. " +
	"Usage uses <placeholders> for values and [brackets] for optional arguments. " +
	"Quote values containing spaces. " +
	"Stdin bodies use a quoted heredoc, pipe, or input redirection; they have no command-line option. " +
	"Use --name=value for option values beginning with --, and -- before positional values beginning with --."

// Help renders this group and every command below it.
func (g *Group[B]) Help(path string) string {
	lines := []string{path + ": " + g.Summary, "", "Subcommands:"}
	for _, child := range g.Subcommands {
		lines = append(lines, fmt.Sprintf("  %s %s — %s", path, Name(child), Summary(child)))
	}
	blocks := []string{strings.Join(lines, "\n")}
	for _, child := range g.Subcommands {
		blocks = append(blocks, child.Help(path+" "+Name(child)))
	}
	return strings.Join(blocks, "\n\n")
}

// Help renders the command's invocation, parameters and output documentation.
func (l *Leaf[B]) Help(path string) string {
	lines := []string{path + ": " + l.Summary, "", "Usage:", ""}
	arguments := l.helpArguments()
	properties := l.properties()
	invocation := strings.Join(append([]string{path}, arguments...), " ")
	if l.StdinField != nil {
		lines = append(lines, "  "+invocation+" <<'EOF'", "  <"+*l.StdinField+">", "  EOF")
	} else {
		lines = append(lines, "  "+invocation)
	}
	if l.SuccessOutput != nil && *l.SuccessOutput != "" {
		lines = append(lines, "    Success output: "+*l.SuccessOutput)
	} else if l.JSONOutput() != nil {
		lines = append(lines, "    Success output: raw text by default; machine-readable JSON when --json is passed")
	}
	if l.FailureOutput != nil && *l.FailureOutput != "" {
		lines = append(lines, "    Failure output: "+*l.FailureOutput)
	}
	var parameters []string
	for _, field := range l.propertyNames() {
		if l.StdinField != nil && *l.StdinField == field {
			continue
		}
		source := l.source(field)
		required := "optional"
		if l.Required(field) {
			required = "required"
		}
		if source == sourceOption && schemaType(properties[field]) == "array" {
			required += ", repeatable"
		}
		parameters = append(
			parameters,
			fmt.Sprintf(
				"      %s (%s)%s",
				fieldSyntax(field, properties[field], source),
				required,
				fieldDescription(properties[field]),
			),
		)
	}
	if len(parameters) != 0 {
		lines = append(lines, "    Parameters:")
		lines = append(lines, parameters...)
	}
	if l.StdinField != nil {
		lines = append(lines, "    Stdin body: "+*l.StdinField+fieldDescription(properties[*l.StdinField]))
	}
	if l.JSONOutput() != nil {
		lines = append(lines, "    --json: emits machine-readable JSON for this command")
	}
	return strings.Join(lines, "\n")
}

type inputSource uint8

const (
	sourcePositional inputSource = iota
	sourceStdin
	sourceRest
	sourceOption
)

// source chooses the command-line spelling of an input field.
func (l *Leaf[B]) source(field string) inputSource {
	if l.StdinField != nil && *l.StdinField == field {
		return sourceStdin
	}
	if l.RestField != nil && *l.RestField == field {
		return sourceRest
	}
	if slices.Contains(l.positionals(), field) {
		return sourcePositional
	}
	return sourceOption
}

// fieldSyntax renders a field in its declared input-source form.
func fieldSyntax(field string, schema any, source inputSource) string {
	if source == sourcePositional {
		return "<" + field + ">"
	}
	if source == sourceRest {
		return "-- <" + field + ">..."
	}
	if schemaType(schema) == "boolean" {
		return "--" + field + " [true|false]"
	}
	label := field
	object, _ := schema.(map[string]any)
	if values, ok := object["enum"].([]any); ok {
		labels := make([]string, len(values))
		for i, value := range values {
			if text, ok := value.(string); ok {
				labels[i] = text
			} else {
				// A compiled JSON schema's enum values are valid JSON and always marshal.
				encoded, _ := contract.EncodeJSON(value)
				labels[i] = string(encoded)
			}
		}
		label = strings.Join(labels, "|")
	}
	return "--" + field + " <" + label + ">"
}

// fieldDescription appends a field's optional schema documentation.
func fieldDescription(schema any) string {
	object, _ := schema.(map[string]any)
	description, _ := object["description"].(string)
	if description == "" {
		return ""
	}
	return " - " + description
}

func (l *Leaf[B]) helpArguments() []string {
	fields := slices.Clone(l.positionals())
	for _, field := range l.propertyNames() {
		if l.source(field) == sourceOption {
			fields = append(fields, field)
		}
	}
	if l.RestField != nil {
		fields = append(fields, *l.RestField)
	}
	arguments := make([]string, 0, len(fields)+1)
	properties := l.properties()
	for _, field := range fields {
		syntax := fieldSyntax(field, properties[field], l.source(field))
		if !l.Required(field) {
			syntax = "[" + syntax + "]"
		}
		arguments = append(arguments, syntax)
	}
	if l.JSONOutput() != nil {
		index := len(arguments)
		if l.RestField != nil {
			index--
		}
		arguments = slices.Insert(arguments, index, "[--json]")
	}
	return arguments
}

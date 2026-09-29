package commandtree

import (
	"fmt"
	"slices"
	"strings"
)

// HelpDefaults is the paragraph the model's command help opens with: what every
// command does unless its own help says otherwise (docs/execution/commands.md,
// Help).
const HelpDefaults = "Unless a command states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero. Pass --help at any level to print a command's documentation. Usage uses <placeholders> for values and [brackets] for optional arguments. Quote values containing spaces. Stdin bodies use a quoted heredoc, pipe, or input redirection; they have no command-line option. Use --name=value for option values beginning with --, and -- before positional values beginning with --."

// Help renders the help of a node and, for a group, of every node below it;
// path is the command line that names the node.
func Help(node Node, path string) string {
	lines := []string{path + ": " + Summary(node)}
	if leaf, ok := node.(Leaf); ok {
		lines = append(lines, "", "Usage:", "")
		lines = append(lines, leafHelp(leaf, path)...)
	}
	group, ok := node.(Group)
	if !ok {
		return strings.Join(lines, "\n")
	}
	lines = append(lines, "", "Subcommands:")
	for _, child := range group.Subcommands {
		lines = append(lines, "  "+path+" "+Name(child)+" — "+Summary(child))
	}
	blocks := []string{strings.Join(lines, "\n")}
	for _, child := range group.Subcommands {
		blocks = append(blocks, Help(child, path+" "+Name(child)))
	}
	return strings.Join(blocks, "\n\n")
}

// leafHelp returns the lines of a command's usage, and of its output and
// parameters.
func leafHelp(leaf Leaf, path string) []string {
	properties, _ := leaf.Properties()
	arguments := slices.Clone(valueOf(leaf.Positionals))
	for field := range properties.All() {
		if sourceOf(leaf, field) == sourceOption {
			arguments = append(arguments, field)
		}
	}
	if leaf.RestField != nil {
		arguments = append(arguments, *leaf.RestField)
	}
	syntaxes := make([]string, len(arguments))
	for i, field := range arguments {
		schema, _ := properties.Get(field)
		syntaxes[i] = syntax(field, schema, sourceOf(leaf, field))
		if !leaf.IsRequired(field) {
			syntaxes[i] = "[" + syntaxes[i] + "]"
		}
	}
	if leaf.JSONOutput() != nil {
		index := len(syntaxes)
		if leaf.RestField != nil {
			index--
		}
		syntaxes = append(syntaxes[:index], append([]string{"[--json]"}, syntaxes[index:]...)...)
	}
	invocation := strings.Join(append([]string{path}, syntaxes...), " ")
	var lines []string
	if leaf.StdinField != nil {
		lines = append(lines, "  "+invocation+" <<'EOF'", "  <"+*leaf.StdinField+">", "  EOF")
	} else {
		lines = append(lines, "  "+invocation)
	}
	switch {
	case leaf.SuccessOutput != nil && *leaf.SuccessOutput != "":
		lines = append(lines, "    Success output: "+*leaf.SuccessOutput)
	case leaf.JSONOutput() != nil:
		lines = append(lines, "    Success output: raw text by default; machine-readable JSON when --json is passed")
	}
	if leaf.FailureOutput != nil && *leaf.FailureOutput != "" {
		lines = append(lines, "    Failure output: "+*leaf.FailureOutput)
	}
	if _, ok := leaf.Properties(); ok {
		lines = append(lines, parameterLines(leaf, properties)...)
	}
	if leaf.JSONOutput() != nil {
		lines = append(lines, "    --json: emits machine-readable JSON for this command")
	}
	return lines
}

// parameterLines returns the lines that describe each parameter and the stdin
// body.
func parameterLines(leaf Leaf, properties Object) []string {
	var lines []string
	first := true
	for field, schema := range properties.All() {
		if leaf.StdinField != nil && *leaf.StdinField == field {
			continue
		}
		if first {
			lines = append(lines, "    Parameters:")
			first = false
		}
		origin := sourceOf(leaf, field)
		required := "optional"
		if leaf.IsRequired(field) {
			required = "required"
		}
		repeatable := ""
		if origin == sourceOption && typeOf(schema) == "array" {
			repeatable = ", repeatable"
		}
		lines = append(lines, fmt.Sprintf("      %s (%s%s)%s", syntax(field, schema, origin), required, repeatable, description(schema)))
	}
	if leaf.StdinField != nil {
		schema, _ := properties.Get(*leaf.StdinField)
		lines = append(lines, "    Stdin body: "+*leaf.StdinField+description(schema))
	}
	return lines
}

// syntax returns how a field is written in a command line.
func syntax(field string, schema any, origin source) string {
	switch {
	case origin == sourcePositional:
		return "<" + field + ">"
	case origin == sourceRest:
		return "-- <" + field + ">..."
	case typeOf(schema) == "boolean":
		return "--" + field + " [true|false]"
	}
	label := field
	if values, ok := lookup(schema, []string{"enum"}).([]any); ok {
		choices := make([]string, len(values))
		for i, value := range values {
			if text, isText := value.(string); isText {
				choices[i] = text
			} else {
				choices[i] = encodeValue(value)
			}
		}
		label = strings.Join(choices, "|")
	}
	return "--" + field + " <" + label + ">"
}

// description returns the text that follows a parameter: its description, when
// it has one.
func description(schema any) string {
	text, _ := lookup(schema, []string{"description"}).(string)
	if text == "" {
		return ""
	}
	return " - " + text
}

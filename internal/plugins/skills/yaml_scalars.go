package skills

import (
	"fmt"
	"strings"

	"github.com/goccy/go-yaml"
	"github.com/goccy/go-yaml/ast"
	"github.com/goccy/go-yaml/parser"
	"github.com/goccy/go-yaml/token"
)

// skillScalar returns a scalar node as written, with its tag, for the skill's own decoders:
// go-yaml's ordinary string decoder normalizes numbers (001 becomes 1), and
// its bool decoder refuses the YAML 1.1 forms (yes, no, y, n, on, off) and quoted booleans, which decodeSkillBool
// accepts.
// Its custom-decoder API supplies alias-resolved YAML; its parser and AST
// remain responsible for all YAML syntax, tags, escaping and block scalars.
func skillScalar(data []byte) (ast.Node, string, error) {
	file, err := parser.ParseBytes(data, 0)
	if err != nil {
		return nil, "", err
	}
	if len(file.Docs) != 1 || file.Docs[0].Body == nil {
		return nil, "", fmt.Errorf("expected a scalar")
	}
	node := file.Docs[0].Body
	tag := ""
	for {
		switch value := node.(type) {
		case *ast.TagNode:
			tag = value.Start.Value
			node = value.Value
		case *ast.AnchorNode:
			node = value.Value
		default:
			return node, tag, nil
		}
	}
}

func decodeSkillText(value *string, data []byte) error {
	node, tag, err := skillScalar(data)
	if err != nil {
		return err
	}
	switch tag {
	case string(token.BinaryTag):
		// Delegate base64 handling to the YAML library as for ordinary tagged data.
		return yaml.Unmarshal(data, value)
	case "!!int", "!!float", "!!bool", "!!null", "!!seq", "!!map", "!!timestamp":
		return fmt.Errorf("tag %s cannot deserialize into string", tag)
	}
	switch node.Type() {
	case ast.IntegerType, ast.FloatType, ast.BoolType, ast.InfinityType, ast.NanType:
		*value = node.GetToken().Value
		return nil
	case ast.NullType:
		if tag == "!!str" || tag == "!" {
			*value = node.GetToken().Value
			return nil
		}
	}
	return yaml.NodeToValue(node, value)
}

func decodeSkillBool(value *bool, data []byte) error {
	node, tag, err := skillScalar(data)
	if err != nil {
		return err
	}
	switch tag {
	case "!!str", "!!int", "!!float", "!!null", "!!seq", "!!map":
		return fmt.Errorf("tag %s cannot deserialize into boolean", tag)
	}
	var text string
	if err := yaml.NodeToValue(node, &text); err != nil {
		return err
	}
	switch strings.ToLower(strings.TrimSpace(text)) {
	case "true", "yes", "y", "on":
		*value = true
	case "false", "no", "n", "off":
		*value = false
	default:
		return fmt.Errorf("invalid YAML 1.1 bool: `%s`", text)
	}
	return nil
}

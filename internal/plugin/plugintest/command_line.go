package plugintest

import (
	"strings"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/plugin"
)

// Roots places manifest commands as the plugin host does and pins native
// operations to a dummy descriptor for command-line tests.
func Roots(manifest plugin.Manifest) ([]cmddecl.Node[cmddecl.Binding], error) {
	demi := []cmddecl.Node[cmddecl.NativeOperation]{}
	roots := []cmddecl.Node[cmddecl.NativeOperation]{}
	for _, commands := range manifest.Commands {
		switch commands.Placement {
		case plugin.PlacementDemi:
			demi = append(demi, commands.Tree.Node)
		case plugin.PlacementRoot:
			roots = append(roots, commands.Tree.Node)
		}
	}
	if len(demi) > 0 {
		roots = append(
			[]cmddecl.Node[cmddecl.NativeOperation]{
				&cmddecl.Group[cmddecl.NativeOperation]{
					Name:        plugin.DemiRoot,
					Summary:     plugin.DemiSummary,
					Subcommands: demi,
				},
			},
			roots...)
	}
	result := make([]cmddecl.Node[cmddecl.Binding], 0, len(roots))
	for _, root := range roots {
		pinned, err := cmddecl.Pin(
			root,
			func(cmddecl.NativeOperation) (string, error) {
				return strings.Repeat("0", 64), nil
			},
		)
		if err != nil {
			return nil, err
		}
		result = append(result, pinned)
	}
	return result, nil
}

// Parse validates command-line input, with stdin as the optional body.
func Parse(root cmddecl.Node[cmddecl.Binding], line []string, stdin *string) (*cmddecl.Parsed, error) {
	selected, err := root.Select(line)
	if err != nil {
		return nil, err
	}
	parsed, err := selected.Parse(line)
	if err != nil {
		return nil, err
	}
	if leaf, ok := selected.Node.(*cmddecl.Leaf[cmddecl.Binding]); ok && !parsed.Help {
		return parsed.Validate(leaf, stdin)
	}
	return parsed, nil
}

// Help renders the command selected by line.
func Help(root cmddecl.Node[cmddecl.Binding], line []string) (string, error) {
	selected, err := root.Select(line)
	if err != nil {
		return "", err
	}
	return selected.Node.Help(strings.Join(selected.Path, " ")), nil
}

package plugintest_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
)

// TestCommandLine checks the test adapter's placement, native pinning and
// help path against the same declarations used by plugin registration.
func TestCommandLine(t *testing.T) {
	node, err := cmddecl.DecodeDeclaration(
		[]byte(
			`{"name":"file","summary":"Files","subcommands":[{"name":"read","summary":"Read",` +
				`"kind":"native","binding":{"package":"demi.file","operation":"read"}}]}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	roots, err := plugintest.Roots(
		plugin.Manifest{
			Commands: []plugin.Commands{{Placement: plugin.PlacementDemi, Tree: plugin.Declaration{Node: node}}},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(roots) != 1 {
		t.Fatalf("roots = %d", len(roots))
	}
	parsed, err := plugintest.Parse(roots[0], []string{"file", "read"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(parsed.Path, " ") != "demi file read" {
		t.Fatalf("path = %v", parsed.Path)
	}
	binding, native := roots[0].Leaves()[0].Binding()
	if !native || binding.DescriptorHash != strings.Repeat("0", 64) {
		t.Fatalf("binding = %#v", binding)
	}
	help, err := plugintest.Help(roots[0], []string{"file", "read"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(help, "demi file read") {
		t.Fatalf("help = %s", help)
	}
}

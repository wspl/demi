package plugintest

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
)

// TestCommandLine checks the test adapter's placement, native pinning and
// help path against the same declarations used by plugin registration.
func TestCommandLine(t *testing.T) {
	node, err := declare.DecodeDeclaration([]byte(`{"name":"file","summary":"Files","subcommands":[{"name":"read","summary":"Read","kind":"native","binding":{"package":"demi.file","operation":"read"}}]}`))
	if err != nil {
		t.Fatal(err)
	}
	roots, err := Roots(plugin.Manifest{Commands: []plugin.Commands{{Placement: plugin.PlacementDemi, Tree: plugin.Declaration{Node: node}}}})
	if err != nil {
		t.Fatal(err)
	}
	if len(roots) != 1 {
		t.Fatalf("roots = %d", len(roots))
	}
	parsed, err := Parse(roots[0], []string{"file", "read"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(parsed.Path, " ") != "demi file read" {
		t.Fatalf("path = %v", parsed.Path)
	}
	binding := roots[0].Leaves()[0].Binding()
	if binding == nil || binding.DescriptorHash != strings.Repeat("0", 64) {
		t.Fatalf("binding = %#v", binding)
	}
	help, err := Help(roots[0], []string{"file", "read"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(help, "demi file read") {
		t.Fatalf("help = %s", help)
	}
}

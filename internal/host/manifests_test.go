package host_test

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
)

// These golden declaration trees pin declaration-builder fidelity against actual plugin manifests.
// They are extracted without changes from the built-in plugins' captured manifests.
func TestBuildersPreservePluginDeclarations(t *testing.T) {
	paths, err := filepath.Glob("testdata/*.json")
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) == 0 {
		t.Fatal("no manifest fixtures")
	}
	for _, path := range paths {
		t.Run(filepath.Base(path), func(t *testing.T) {
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			tree, err := declare.DecodeDeclaration(data)
			if err != nil {
				t.Fatal(err)
			}
			declared := rebuildDeclaration(t, tree)
			var commands host.CommandSet
			if err = commands.Register(host.Group("demi", "Demi.", declared)); err != nil {
				t.Fatal(err)
			}
			actual, err := contract.EncodeJSON(
				commands.Declarations()[0].(*declare.Group[declare.NativeOperation]).Subcommands[0],
			)
			if err != nil {
				t.Fatal(err)
			}
			var want, got any
			if err = json.Unmarshal(data, &want); err != nil {
				t.Fatal(err)
			}
			if err = json.Unmarshal(actual, &got); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("builder changed golden manifest\n%s", actual)
			}
		})
	}
}

// rebuildDeclaration uses Go declaration constructors and the browser's dynamic timeout description rule.
func rebuildDeclaration(t *testing.T, node declare.Node[declare.NativeOperation]) host.Declared {
	t.Helper()
	switch node := node.(type) {
	case *declare.Group[declare.NativeOperation]:
		children := make([]host.Declared, 0, len(node.Subcommands))
		for _, child := range node.Subcommands {
			children = append(children, rebuildDeclaration(t, child))
		}
		return host.Group(node.Name, node.Summary, children...)
	case *declare.Leaf[declare.NativeOperation]:
		var handler host.RPCHandler
		if _, rpc := node.Kind.(*declare.RPC[declare.NativeOperation]); rpc {
			handler = host.RPCHandlerFunc(
				func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) { return 0, nil },
			)
		}
		declared := host.Leaf(*node, handler)
		if node.Input != nil {
			// The browser plugin adds a leaf-specific default to this field's type documentation.
			properties := node.Properties()
			if timeout, ok := properties["timeout"].(map[string]any); ok {
				if description, ok := timeout["description"].(string); ok &&
					strings.HasPrefix(description, "Whole operation deadline in milliseconds; default ") {
					declared = declared.Describe("timeout", "Whole operation deadline in milliseconds")
					declared = declared.Describe("timeout", description)
				}
			}
		}
		return declared
	}
	t.Fatal("unknown node")
	return host.Declared{}
}

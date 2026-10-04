package cmddecl_test

import (
	"encoding/json"
	"testing"

	"github.com/wspl/demi/internal/cmddecl"
)

// Boundary conversion cases run locally within one second, without IO or timers.
func TestDeclarationAndManifestBindingFamilies(t *testing.T) {
	const operation = `{"package":"demi.file","operation":"read"}`
	const binding = `{"package":"demi.file","operation":"read","descriptorHash":"abc"}`
	for _, test := range []struct {
		name, document        string
		declaration, manifest bool
	}{
		{"rpc", `{"name":"x","summary":"X","kind":"rpc"}`, true, true},
		{"declaration", `{"name":"x","summary":"X","kind":"native","binding":` + operation + `}`, true, false},
		{"manifest", `{"name":"x","summary":"X","kind":"native","binding":` + binding + `}`, false, true},
		{"nested declaration", `{"name":"g","summary":"G","subcommands":[{"name":"x","summary":"X","kind":"native",` +
			`"binding":` + operation + `}]}`, true, false},
		{"nested manifest", `{"name":"g","summary":"G","subcommands":[{"name":"x","summary":"X","kind":"native",` +
			`"binding":` + binding + `}]}`, false, true},
		{"missing binding", `{"name":"x","summary":"X","kind":"native"}`, false, false},
		{"rpc binding", `{"name":"x","summary":"X","kind":"rpc","binding":` + binding + `}`, false, false},
		{"unknown kind", `{"name":"x","summary":"X","kind":"other"}`, false, false},
		{"null binding", `{"name":"x","summary":"X","kind":"rpc","binding":null}`, false, false},
		{"null schema", `{"name":"x","summary":"X","input":null,"kind":"rpc"}`, false, false},
		{"nonobject schema", `{"name":"x","summary":"X","input":true,"kind":"rpc"}`, false, false},
		{"bad schema", `{"name":"x","summary":"X","input":{"type":"invalid"},"kind":"rpc"}`, false, false},
		{"bad output schema", `{"name":"x","summary":"X","output":{"json":{"type":"invalid"}},"kind":"rpc"}`, false, false},
		{"empty output and positionals", `{"name":"x","summary":"X","positionals":[],"output":{},"kind":"rpc"}`, true, true},
		{"empty group", `{"name":"g","summary":"G","subcommands":[]}`, true, true},
		{"absent subcommands", `{"name":"g","summary":"G"}`, false, false},
		{"mixed variants", `{"name":"g","summary":"G","subcommands":[],"kind":"rpc"}`, false, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			declaration, err := cmddecl.DecodeDeclaration([]byte(test.document))
			if (err == nil) != test.declaration {
				t.Fatalf("declaration decode: %v", err)
			}
			if test.declaration {
				encoded, err := json.Marshal(declaration)
				if err != nil || string(encoded) != test.document {
					t.Fatalf("declaration round trip: %s, %v", encoded, err)
				}
			}
			manifest, err := cmddecl.DecodeManifestNode([]byte(test.document))
			if (err == nil) != test.manifest {
				t.Fatalf("manifest decode: %v", err)
			}
			if test.manifest {
				encoded, err := json.Marshal(manifest)
				if err != nil || string(encoded) != test.document {
					t.Fatalf("manifest round trip: %s, %v", encoded, err)
				}
			}
		})
	}
}

func TestDecodedSchemasAndPinning(t *testing.T) {
	const document = `{"name":"x","summary":"X","input":{"type":"object","properties":{"n":{"type":"integer"}}},` +
		`"output":{"json":{"type":"integer"}},"kind":"native","binding":{"package":"demi.file",` +
		`"operation":"read"}}`
	node, err := cmddecl.DecodeDeclaration([]byte(document))
	if err != nil {
		t.Fatal(err)
	}
	pinned, err := cmddecl.Pin(node, func(cmddecl.NativeOperation) (string, error) {
		return "abc", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(pinned)
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := cmddecl.DecodeManifestNode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	leaf, ok := manifest.(*cmddecl.Leaf[cmddecl.Binding])
	if !ok {
		t.Fatalf("decoded manifest is not a leaf: %T", manifest)
	}
	if err := leaf.CheckArguments(
		json.RawMessage(`{"n":"bad"}`),
	); err == nil ||
		err.Error() != `Invalid command arguments: "n" is not of type "integer"` {
		t.Fatalf("decoded input schema: %v", err)
	}
	if err := leaf.JSONOutput().
		Check(json.RawMessage(`"bad"`)); err == nil ||
		err.Error() != `value is not of type "integer"` {
		t.Fatalf("decoded output schema: %v", err)
	}
	for _, test := range []struct{ document, want string }{
		{`{"name":"x","summary":"X","kind":"native"}`, "a native command names its binding"},
		{`{"name":"x","summary":"X","kind":"rpc","binding":{"package":"p","operation":"o",` +
			`"descriptorHash":"h"}}`, "an rpc command has no binding"},
	} {
		if _, err := cmddecl.DecodeManifestNode([]byte(test.document)); err == nil || err.Error() != test.want {
			t.Fatalf("pairing refusal: %v, want %s", err, test.want)
		}
	}
}

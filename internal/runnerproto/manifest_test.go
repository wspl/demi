package runnerproto_test

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerproto"
)

func manifestFixture(t *testing.T) []byte {
	t.Helper()
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestManifestVerification(t *testing.T) {
	data := manifestFixture(t)
	manifest, err := runnerproto.DecodeManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := manifest.Roots["fixture"]; !ok {
		t.Fatal("fixture root missing")
	}
	for _, tc := range []struct{ name, from, to, reason string }{
		{"changed summary", `CLI fixture.`, `corrupt`, "hash mismatch"},
		{"duplicate operation", `"fixture.echo"`, `"file.read"`, "duplicate"},
		{"contradictory input", `"positionals": [
              "path"
            ]`, `"positionals":["path","body"]`, "multiple input sources for body"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			corrupted := bytes.Replace(data, []byte(tc.from), []byte(tc.to), 1)
			if bytes.Equal(data, corrupted) {
				t.Fatal("mutation did not change fixture")
			}
			_, err := runnerproto.DecodeManifest(corrupted)
			if err == nil || !strings.Contains(err.Error(), tc.reason) {
				t.Fatalf("error=%v, want %s", err, tc.reason)
			}
		})
	}
}

// declarations unpins the fixture through declare's typed trees rather than
// declaring another copy of a manifest node's wire shape.
func declarations(t *testing.T, manifest runnerproto.Manifest) []cmddecl.Node[cmddecl.NativeOperation] {
	t.Helper()
	var roots []cmddecl.Node[cmddecl.NativeOperation]
	for _, root := range manifest.Roots {
		node, err := cmddecl.DecodeManifestNode(root.Tree)
		if err != nil {
			t.Fatal(err)
		}
		roots = append(roots, unpin(node))
	}
	return roots
}

func unpin(node cmddecl.Node[cmddecl.Binding]) cmddecl.Node[cmddecl.NativeOperation] {
	switch node := node.(type) {
	case *cmddecl.Group[cmddecl.Binding]:
		children := make([]cmddecl.Node[cmddecl.NativeOperation], 0, len(node.Subcommands))
		for _, child := range node.Subcommands {
			children = append(children, unpin(child))
		}
		return &cmddecl.Group[cmddecl.NativeOperation]{
			Name:        node.Name,
			Summary:     node.Summary,
			Subcommands: children,
		}
	case *cmddecl.Leaf[cmddecl.Binding]:
		leaf := &cmddecl.Leaf[cmddecl.NativeOperation]{
			Name:          node.Name,
			Summary:       node.Summary,
			SuccessOutput: node.SuccessOutput,
			FailureOutput: node.FailureOutput,
			RunningHint:   node.RunningHint,
			Input:         node.Input,
			Positionals:   node.Positionals,
			StdinField:    node.StdinField,
			RestField:     node.RestField,
			Output:        node.Output,
		}
		if binding, native := node.Binding(); native {
			leaf.Kind = &cmddecl.Native[cmddecl.NativeOperation]{
				Binding: cmddecl.NativeOperation{Package: binding.Package, Operation: binding.Operation},
			}
		} else {
			leaf.Kind = &cmddecl.RPC[cmddecl.NativeOperation]{}
		}
		return leaf
	}
	return nil
}

func TestManifestBuild(t *testing.T) {
	data := manifestFixture(t)
	recorded, err := runnerproto.DecodeManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	packages := make([]cmdproto.PackageDescriptor, 0, len(recorded.Packages))
	for _, descriptor := range recorded.Packages {
		packages = append(packages, descriptor)
	}
	roots := declarations(t, recorded)
	built, err := runnerproto.BuildManifest(roots, packages)
	if err != nil {
		t.Fatal(err)
	}
	if built.Hash != recorded.Hash {
		t.Fatalf("hash=%s, want fixture %s", built.Hash, recorded.Hash)
	}
	encoded, err := contract.EncodeJSON(built)
	if err != nil {
		t.Fatal(err)
	}
	// JSON formatting and map key order are not part of the canonical digest.
	var got, want any
	if err := json.Unmarshal(encoded, &got); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &want); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatal("built manifest differs from fixture")
	}
	native := func(pkg, op string) cmddecl.Node[cmddecl.NativeOperation] {
		return &cmddecl.Leaf[cmddecl.NativeOperation]{
			Name:    "native",
			Summary: "Native",
			Kind: &cmddecl.Native[cmddecl.NativeOperation]{
				Binding: cmddecl.NativeOperation{Package: pkg, Operation: op},
			},
		}
	}
	rpc := &cmddecl.Leaf[cmddecl.NativeOperation]{
		Name:    "rpc",
		Summary: "Rpc",
		Kind:    &cmddecl.RPC[cmddecl.NativeOperation]{},
	}
	contradictory, err := cmddecl.DecodeDeclaration(
		[]byte(
			`{"name":"note","summary":"Note","kind":"rpc","input":{"type":"object",` +
				`"properties":{"text":{"type":"string"}}},"positionals":["text"],"stdinField":"text"}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name     string
		roots    []cmddecl.Node[cmddecl.NativeOperation]
		packages []cmdproto.PackageDescriptor
		reason   string
	}{
		{
			"missing package",
			[]cmddecl.Node[cmddecl.NativeOperation]{
				native("demicodes.other", "file.read"),
			},
			packages,
			"not configured",
		},
		{
			"missing operation",
			[]cmddecl.Node[cmddecl.NativeOperation]{
				native("demicodes.fixture", "file.gone"),
			},
			packages,
			"no operation",
		},
		{"duplicate root", []cmddecl.Node[cmddecl.NativeOperation]{rpc, rpc}, nil, "duplicate root"},
		{
			"duplicate package",
			nil,
			append(append([]cmdproto.PackageDescriptor{}, packages...), packages...),
			"duplicate command package",
		},
		{
			"contradictory input",
			[]cmddecl.Node[cmddecl.NativeOperation]{
				contradictory,
			},
			nil,
			"multiple input sources for text",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := runnerproto.BuildManifest(tc.roots, tc.packages)
			if err == nil || !strings.Contains(err.Error(), tc.reason) {
				t.Fatalf("error=%v, want %s", err, tc.reason)
			}
		})
	}
	first := "working"
	second := "still working"
	rpc.RunningHint = &first
	a, err := runnerproto.BuildManifest([]cmddecl.Node[cmddecl.NativeOperation]{rpc}, packages)
	if err != nil {
		t.Fatal(err)
	}
	rpc.RunningHint = &second
	b, err := runnerproto.BuildManifest([]cmddecl.Node[cmddecl.NativeOperation]{rpc}, packages)
	if err != nil {
		t.Fatal(err)
	}
	if a.Hash == b.Hash {
		t.Fatal("hash excludes running hint")
	}
	packages[0].Version = "next"
	c, err := runnerproto.BuildManifest([]cmddecl.Node[cmddecl.NativeOperation]{rpc}, packages)
	if err != nil {
		t.Fatal(err)
	}
	if b.Hash == c.Hash {
		t.Fatal("hash excludes descriptor version")
	}
}

func TestManifestBindingAndRootRefusals(t *testing.T) {
	original, err := runnerproto.DecodeManifest(manifestFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	// Recompute the outer hash so verification must reject the corrupted binding itself.
	for _, field := range []string{"package", "operation", "descriptorHash"} {
		t.Run(field, func(t *testing.T) {
			root := original.Roots["fixture"]
			tree, err := cmddecl.DecodeManifestNode(root.Tree)
			if err != nil {
				t.Fatal(err)
			}
			binding := &tree.Leaves()[0].Kind.(*cmddecl.Native[cmddecl.Binding]).Binding
			switch field {
			case "package":
				binding.Package = "demicodes.other"
			case "operation":
				binding.Operation = "gone"
			case "descriptorHash":
				binding.DescriptorHash = strings.Repeat("0", 64)
			}
			raw, err := contract.EncodeJSON(tree)
			if err != nil {
				t.Fatal(err)
			}
			changed := original
			changed.Roots = map[string]runnerproto.Root{"fixture": {Tree: raw}}
			changed.Hash, err = cmdproto.CanonicalDigest(
				map[string]any{"roots": changed.Roots, "packages": changed.Packages},
			)
			if err != nil {
				t.Fatal(err)
			}
			if err := changed.Validate(); err == nil || !strings.Contains(err.Error(), "unresolved") {
				t.Fatalf("binding refusal: %v", err)
			}
		})
	}
	wrong := original
	wrong.Roots = map[string]runnerproto.Root{"wrong": original.Roots["fixture"]}
	if err := wrong.Validate(); err == nil || !strings.Contains(err.Error(), "root name mismatch") {
		t.Fatalf("name refusal: %v", err)
	}
}

func TestManifestJSONThroughWire(t *testing.T) {
	data := manifestFixture(t)
	encoded, err := runnerproto.Encode(&runnerproto.ManifestMessage{Manifest: data})
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := runnerproto.DecodeInbound(encoded)
	if err != nil {
		t.Fatal(err)
	}
	envelope, ok := decoded.(*runnerproto.ManifestMessage)
	if !ok {
		t.Fatalf("manifest=%T", decoded)
	}
	// The JSON fixture is pretty-printed, whereas MessagePack carries no
	// whitespace. Restore the fixture's indentation to compare every JSON byte,
	// including its original object order, through the actual opaque wire field.
	var pretty bytes.Buffer
	if err := json.Indent(&pretty, envelope.Manifest, "", "  "); err != nil {
		t.Fatal(err)
	}
	pretty.WriteByte('\n')
	if !bytes.Equal(pretty.Bytes(), data) {
		t.Fatalf("manifest JSON bytes changed\n%s", pretty.Bytes())
	}
	verified, err := runnerproto.DecodeManifest(envelope.Manifest)
	if err != nil {
		t.Fatal(err)
	}
	if verified.Hash != "0cf18d78aae679a7d2e1af76e21e2f1d4c27a7e3a56ad6394a87b5b6229b0235" {
		t.Fatalf("fixture hash changed: %s", verified.Hash)
	}
}

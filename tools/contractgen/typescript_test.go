package main

import (
	"go/token"
	"go/types"
	"strings"
	"testing"
)

// Cost: in-memory declarations; preserves the Rust Zod recursion and scalar assertions.
func TestRecursiveTypeScriptUsesGetter(t *testing.T) {
	pkg := types.NewPackage("fixture/tree", "tree")
	tree := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Tree", nil), nil, nil)
	tree.SetUnderlying(types.NewStruct([]*types.Var{
		types.NewVar(token.NoPos, pkg, "Name", types.Typ[types.String]),
		types.NewVar(token.NoPos, pkg, "Children", types.NewSlice(tree)),
	}, []string{`json:"name"`, `json:"children"`}))
	key := typeKey(tree)
	g := generator{defs: map[string]*definition{key: {name: "Tree", key: key, typ: tree, marks: map[string]string{"root": "direction=receive output=protocol"}, fields: map[string]map[string]string{}}}, order: []string{key}}
	sources, err := g.tsSources()
	if err != nil {
		t.Fatal(err)
	}
	source := string(sources["protocol"])
	for _, want := range []string{`get "children"() { return z.array(treeSchema) }`, `"name": z.string()`} {
		if !strings.Contains(source, want) {
			t.Fatalf("missing %s in %s", want, source)
		}
	}
	if strings.Count(source, "import ") != 1 || !strings.Contains(source, "from 'zod'") {
		t.Fatalf("unexpected schema imports: %s", source)
	}
}

// Cost: in-memory scalar emission; both missing bounds must be refused.
func TestJavaScriptIntegerBounds(t *testing.T) {
	for _, tc := range []struct {
		typ    *types.Basic
		bounds string
		reason string
		want   string
	}{
		{types.Typ[types.Uint64], "min=0", "safe maximum", ""},
		{types.Typ[types.Int64], "max=5", "safe minimum", ""},
		{types.Typ[types.Uint64], "max=9007199254740991", "", "z.int().min(0)"},
		{types.Typ[types.Uint32], "", "", "z.int().min(0).max(4294967295)"},
	} {
		g := generator{}
		source, _ := g.tsType(tc.typ, map[string]string{"range": tc.bounds})
		if tc.reason != "" {
			if g.err == nil || !strings.Contains(g.err.Error(), tc.reason) {
				t.Fatalf("%s %s: %v", tc.typ, tc.bounds, g.err)
			}
		} else if g.err != nil || source != tc.want {
			t.Fatalf("%s: %s %v", tc.typ, source, g.err)
		}
	}
}

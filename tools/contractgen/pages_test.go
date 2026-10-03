package main

import (
	"go/token"
	"go/types"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/tools/contractgen/pagemeta"
)

func fixturePages(t *testing.T) []pagemeta.Page {
	t.Helper()
	data, err := os.ReadFile("testdata/pages/manifests.json")
	if err != nil {
		t.Fatal(err)
	}
	pages, err := pagemeta.Decode(data)
	if err != nil {
		t.Fatal(err)
	}
	return pages
}

// Cost: fixture bytes and two temporary app manifests; no processes or waits.
func TestPageModulesAndRegistries(t *testing.T) {
	pages := fixturePages(t)
	pkg := types.NewPackage("fixture/pages", "pages")
	g := generator{defs: map[string]*definition{}}
	named := map[string]*types.Named{}
	for _, name := range []string{"Shared", "Nested", "State", "Params", "Other"} {
		typ := types.NewNamed(types.NewTypeName(token.NoPos, pkg, name, nil), nil, nil)
		named[name] = typ
		d := &definition{
			name:   name,
			key:    typeKey(typ),
			typ:    typ,
			marks:  map[string]string{},
			fields: map[string]map[string]string{},
		}
		g.defs[d.key] = d
		g.order = append(g.order, d.key)
	}
	named["Shared"].SetUnderlying(types.Typ[types.String])
	g.defs[typeKey(named["Shared"])].marks = map[string]string{
		"root": "direction=receive output=protocol",
		"enum": "one two",
	}
	named["Nested"].SetUnderlying(types.NewStruct(nil, nil))
	named["State"].SetUnderlying(types.NewStruct([]*types.Var{
		types.NewVar(token.NoPos, pkg, "Nested", named["Nested"]),
		types.NewVar(token.NoPos, pkg, "Shared", named["Shared"]),
	}, []string{`json:"nested"`, `json:"shared"`}))
	g.defs[typeKey(named["State"])].marks["root"] = "direction=receive output=plugin-browser"
	named["Params"].SetUnderlying(types.NewStruct([]*types.Var{
		types.NewVar(token.NoPos, pkg, "Nested", named["Nested"]),
	}, []string{`json:"nested"`}))
	g.defs[typeKey(named["Params"])].marks["root"] = "direction=send output=plugin-browser"
	// Receipt by a different boundary must not relax a page's send-only type.
	named["Other"].SetUnderlying(types.NewStruct([]*types.Var{
		types.NewVar(token.NoPos, pkg, "Params", named["Params"]),
	}, []string{`json:"params"`}))
	g.defs[typeKey(named["Other"])].marks["root"] = "direction=receive output=web"

	sources, err := g.tsSources(pages...)
	if err != nil {
		t.Fatal(err)
	}
	if err := g.pageSources(pages, sources); err != nil {
		t.Fatal(err)
	}
	source := string(sources["plugin-browser"])
	for _, want := range []string{
		"/** The plugin this package is the page of. */",
		"export const PLUGIN = \"browser\"",
		"/** A shared stream value. */",
		"export const LIVE_TEST = {\"z\":\"<>&\u2028\u2029\",\"a\":2}",
	} {
		if !strings.Contains(source, want) {
			t.Errorf("page module missing %q: %s", want, source)
		}
	}
	root := t.TempDir()
	for _, app := range []string{"web", "web-gallery"} {
		dir := filepath.Join(root, "packages", app)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(
			filepath.Join(dir, "package.json"),
			[]byte(
				`{"unrelated":true,"dependencies":{`+
					`"@demicodes/plugin-skills":"workspace:^","@demicodes/utils":"workspace:^",`+
					`"@demicodes/plugin-browser":"workspace:^",`+
					`"@demicodes/plugin-file-browser":"workspace:^"}}`,
			),
			0o600,
		); err != nil {
			t.Fatal(err)
		}
	}

	for _, app := range []string{"web", "web-gallery"} {
		selected, err := pagesOf(root, filepath.Join("packages", app), pages)
		if err != nil {
			t.Fatal(err)
		}
		var pairs [][2]string
		for _, page := range selected {
			pairs = append(pairs, [2]string{page.ID, page.Package})
		}
		want := [][2]string{
			{"browser", "@demicodes/plugin-browser"},
			{"skills", "@demicodes/plugin-skills"},
			{"file-browser", "@demicodes/plugin-file-browser"},
		}
		if !reflect.DeepEqual(pairs, want) {
			t.Fatalf("%s pages: %v, want %v", app, pairs, want)
		}
	}
	if err := writeRegistries(root, pages, false); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{
		"packages/web/src/plugins/generated/pages.ts",
		"packages/web-gallery/src/generated/pages.ts",
	} {
		data, err := os.ReadFile(filepath.Join(root, path))
		if err != nil {
			t.Fatal(err)
		}
		for _, want := range []string{
			"import browserPage from \"@demicodes/plugin-browser\"",
			"import fileBrowserPage from \"@demicodes/plugin-file-browser\"",
			"[\n  browserPage,\n  skillsPage,\n  fileBrowserPage,\n]",
		} {
			if !strings.Contains(string(data), want) {
				t.Errorf("%s missing %q: %s", path, want, data)
			}
		}
	}
}

// Cost: in-memory manifest comparisons, under a millisecond.
func TestPageManifestMismatch(t *testing.T) {
	page := fixturePages(t)[0]
	for _, tc := range []struct {
		name     string
		actual   map[string]bool
		typeName string
		reason   string
	}{
		{"missing", map[string]bool{"Nested": true, "Params": false}, "State", "missing"},
		{"extra", map[string]bool{"Nested": true, "Params": false, "State": true, "Extra": false}, "Extra", "extra"},
		{"direction", map[string]bool{"Nested": true, "Params": true, "State": true}, "Params", "direction"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := checkPage(page, tc.actual, map[string]bool{"Shared": true})
			if err == nil {
				t.Fatal("accepted mismatched page")
			}
			for _, want := range []string{"browser", tc.typeName, tc.reason} {
				if !strings.Contains(err.Error(), want) {
					t.Errorf("diagnostic %q missing %q", err, want)
				}
			}
		})
	}
}

func TestPageOutputOwnership(t *testing.T) {
	for _, tc := range []struct {
		name    string
		pages   []pagemeta.Page
		sources map[string][]byte
		want    string
	}{
		{"unowned", nil, map[string][]byte{"plugin-orphan": nil}, "plugin-orphan"},
		{
			"external",
			[]pagemeta.Page{
				{
					ID:      "external",
					Package: "@outside/page",
				},
			},
			nil,
			"the page package @outside/page is not a workspace package of " +
				"@demicodes/",
		},
		{"missing output", fixturePages(t)[:1], map[string][]byte{}, "missing"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			g := generator{}
			err := g.pageSources(tc.pages, tc.sources)
			if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("got %v, want %s", err, tc.want)
			}
		})
	}
}

func TestPackageDependenciesRejectInvalidValues(t *testing.T) {
	for _, data := range []string{
		`null`,
		`{"dependencies":null}`,
		`{"dependencies":[]}`,
		`{"dependencies":{"pkg":null}}`,
		`{"dependencies":{"pkg":2}}`,
		`{} {}`,
	} {
		t.Run(data, func(t *testing.T) {
			root := t.TempDir()
			if err := os.WriteFile(filepath.Join(root, "package.json"), []byte(data), 0o600); err != nil {
				t.Fatal(err)
			}
			if _, err := pagesOf(root, ".", nil); err == nil {
				t.Fatal("accepted invalid package dependencies")
			}
		})
	}
}

// Two different schemas of one name and invalid unnamed roots are refused.
func TestPageSchemaRefusals(t *testing.T) {
	for _, tc := range []struct {
		name    string
		schemas []pagemeta.Schema
		want    string
	}{
		{"unnamed", []pagemeta.Schema{{Direction: "send", Value: []byte(`{"type":"object"}`)}}, "not a named type"},
		{
			"bad definitions",
			[]pagemeta.Schema{
				{
					Direction: "send",
					Value:     []byte(`{"title":"Value","$defs":[]}`),
				},
			},
			"$defs is not an object",
		},
		{
			"null definitions",
			[]pagemeta.Schema{
				{
					Direction: "send",
					Value:     []byte(`{"title":"Value","$defs":null}`),
				},
			},
			"$defs is not an object",
		},
		{"conflicting name", []pagemeta.Schema{
			{Direction: "send", Value: []byte(`{"title":"Value","type":"string"}`)},
			{Direction: "receive", Value: []byte(`{"title":"Value","type":"number"}`)},
		}, "two schemas of Value differ"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := pageTypes(pagemeta.Page{ID: "test", Schemas: tc.schemas}, nil)
			if err == nil || !strings.Contains(err.Error(), tc.want) || !strings.Contains(err.Error(), "test") {
				t.Fatalf("got %v, want plugin and %s", err, tc.want)
			}
		})
	}
}

// A manifest's named scalar is public in its page even when other modules
// factor that scalar privately. Cost: in-memory type model, under a millisecond.
func TestManifestNamedScalarExport(t *testing.T) {
	data, err := os.ReadFile("testdata/pages/scalar.json")
	if err != nil {
		t.Fatal(err)
	}
	pages, err := pagemeta.Decode(data)
	if err != nil {
		t.Fatal(err)
	}
	pkg := types.NewPackage("fixture/scalar", "scalar")
	address := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Address", nil), types.Typ[types.String], nil)
	hidden := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Hidden", nil), types.Typ[types.String], nil)
	state := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "State", nil), types.NewStruct(
		[]*types.Var{
			types.NewVar(token.NoPos, pkg, "Address", address),
			types.NewVar(token.NoPos, pkg, "Hidden", hidden),
		},
		[]string{`json:"address"`, `json:"hidden"`},
	), nil)
	failures := types.NewNamed(
		types.NewTypeName(token.NoPos, pkg, "Failures", nil),
		types.NewMap(types.Typ[types.String], types.Typ[types.String]),
		nil,
	)
	web := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Web", nil), types.NewStruct(
		[]*types.Var{
			types.NewVar(token.NoPos, pkg, "Address", address),
			types.NewVar(token.NoPos, pkg, "Hidden", hidden),
			types.NewVar(token.NoPos, pkg, "Failures", failures),
		},
		[]string{`json:"address"`, `json:"hidden"`, `json:"failures"`},
	), nil)
	g := generator{defs: map[string]*definition{}}
	for _, d := range []*definition{
		{name: "Address", typ: address, marks: map[string]string{"codec": "string"}},
		{name: "Hidden", typ: hidden, marks: map[string]string{"root": "", "schema-primitive": ""}},
		{name: "Failures", typ: failures, marks: map[string]string{}},
		{name: "State", typ: state, marks: map[string]string{"root": "direction=receive output=plugin-scalar"}},
		{name: "Web", typ: web, marks: map[string]string{"root": "direction=receive output=web"}},
	} {
		d.key = typeKey(d.typ)
		g.defs[d.key] = d
		g.order = append(g.order, d.key)
	}
	sources, err := g.tsSources(pages...)
	if err != nil {
		t.Fatal(err)
	}
	if err := g.pageSources(pages, sources); err != nil {
		t.Fatal(err)
	}
	for _, private := range []string{"Hidden", "Failures"} {
		if strings.Contains(string(sources["web"]), "export type "+private) {
			t.Errorf("private factoring leaked into web: %s", private)
		}
	}
	if strings.Contains(string(sources["plugin-scalar"]), "export type Hidden") {
		t.Fatal("Go-only root leaked into page exports")
	}
	for _, declaration := range []string{"export const addressSchema", "export type Address"} {
		if !strings.Contains(string(sources["plugin-scalar"]), declaration) {
			t.Errorf("page missing %s", declaration)
		}
		if strings.Contains(string(sources["web"]), declaration) {
			t.Errorf("page export leaked into web: %s", declaration)
		}
	}
}

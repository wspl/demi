package main

import (
	"go/token"
	"go/types"
	"os"
	"path/filepath"
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
		d := &definition{name: name, key: typeKey(typ), typ: typ, marks: map[string]string{}, fields: map[string]map[string]string{}}
		g.defs[d.key] = d
		g.order = append(g.order, d.key)
	}
	named["Shared"].SetUnderlying(types.Typ[types.String])
	g.defs[typeKey(named["Shared"])].marks = map[string]string{"root": "direction=receive output=protocol", "enum": "one two"}
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

	sources, err := g.tsSources()
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
		if err := os.MkdirAll(dir, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "package.json"), []byte(`{"unrelated":true,"dependencies":{"@demicodes/plugin-skills":"workspace:^","@demicodes/utils":"workspace:^","@demicodes/plugin-browser":"workspace:^","@demicodes/plugin-file-browser":"workspace:^"}}`), 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeRegistries(root, pages, false); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"packages/web/src/plugins/generated/pages.ts", "packages/web-gallery/src/generated/pages.ts"} {
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
		{"external", []pagemeta.Page{{ID: "external", Package: "@outside/page"}}, nil, "the page package @outside/page is not a workspace package of @demicodes/"},
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
	for _, data := range []string{`null`, `{"dependencies":null}`, `{"dependencies":[]}`, `{"dependencies":{"pkg":null}}`, `{"dependencies":{"pkg":2}}`, `{} {}`} {
		t.Run(data, func(t *testing.T) {
			root := t.TempDir()
			if err := os.WriteFile(filepath.Join(root, "package.json"), []byte(data), 0600); err != nil {
				t.Fatal(err)
			}
			if _, err := pagesOf(root, ".", nil); err == nil {
				t.Fatal("accepted invalid package dependencies")
			}
		})
	}
}

// Manifest schema conflicts and invalid unnamed roots are Rust errors too.
func TestPageSchemaRefusals(t *testing.T) {
	for _, tc := range []struct {
		name    string
		schemas []pagemeta.Schema
		want    string
	}{
		{"unnamed", []pagemeta.Schema{{Direction: "send", Value: []byte(`{"type":"object"}`)}}, "not a named type"},
		{"bad definitions", []pagemeta.Schema{{Direction: "send", Value: []byte(`{"title":"Value","$defs":[]}`)}}, "$defs is not an object"},
		{"null definitions", []pagemeta.Schema{{Direction: "send", Value: []byte(`{"title":"Value","$defs":null}`)}}, "$defs is not an object"},
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

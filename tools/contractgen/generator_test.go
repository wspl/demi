package main

import (
	"errors"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/private"
)

// Each package load costs about 0.1 s. Testing actual declarations catches
// diagnostics lost between marker parsing, go/types and emission; budget 5 s.
func TestGenerationRefusals(t *testing.T) {
	paths, err := filepath.Glob("testdata/invalid/*")
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		t.Run(filepath.Base(path), func(t *testing.T) {
			err := generate(t.Context(), []string{"./" + path}, false, "", false)
			if err == nil {
				t.Fatal("accepted unsupported declaration")
			}
			if !strings.Contains(err.Error(), "Broken") || !strings.Contains(err.Error(), "types.go:") {
				t.Fatalf("diagnostic lost type or position: %v", err)
			}
		})
	}
}

// Regeneration checks committed fixtures without writing the checkout.
// This verifies bootstrap and determinism through the CLI's loader.
// Local package loads cost roughly one second; no network or services are used.
func TestRegeneration(t *testing.T) {
	if err := generate(
		t.Context(),
		[]string{
			"./testdata/orderedobject",
			"./testdata/integers",
			"./testdata/private",
			"./testdata/presence",
			"./testdata/codecs",
			"./testdata/blocks",
			"./testdata/runner",
			"./testdata/features",
			"./testdata/text",
			"./testdata/tables",
			"./testdata/kept",
			"./testdata/schemas",
			"./testdata/generics",
			"./testdata/remaining",
			"./testdata/keyed",
			"./testdata/browser",
			"./testdata/reachability",
			"./testdata/packedonly",
			"./testdata/schemacheck",
		},
		false,
		"",
		true,
	); err != nil {
		t.Fatal(err)
	}
	dest := t.TempDir()
	if err := generate(
		t.Context(),
		[]string{"./testdata/blocks", "./testdata/features"},
		true,
		dest,
		false,
	); err != nil {
		t.Fatal(err)
	}
	web, err := os.ReadFile(filepath.Join(dest, "web", "web-api.ts"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(web), "from '@demicodes/protocol'") {
		t.Fatal("shared protocol schema was duplicated")
	}
}

// Package loads exercise imported generic constraints and bootstrap from stale
// or absent output. Uses only temporary local files; budget 20 seconds with
// race instrumentation because each scenario loads and type-checks packages.
func TestGenerationDependencies(t *testing.T) {
	fixture, err := filepath.Abs("testdata/loading")
	if err != nil {
		t.Fatal(err)
	}
	// Stay inside the module so generated imports can reach internal/contract.
	dir, err := os.MkdirTemp(filepath.Dir(fixture), "loading-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.RemoveAll(dir); err != nil {
			t.Error(err)
		}
	})
	for _, pkg := range []string{"dependency", "consumer"} {
		if err := os.Mkdir(filepath.Join(dir, pkg), 0o755); err != nil {
			t.Fatal(err)
		}
		for _, name := range []string{"types.go", "contract_gen.go"} {
			data, err := os.ReadFile(filepath.Join(fixture, pkg, name))
			if err != nil {
				t.Fatal(err)
			}
			data = []byte(
				strings.ReplaceAll(
					string(data),
					"testdata/loading/dependency",
					"testdata/"+filepath.Base(dir)+"/dependency",
				),
			)
			if err := os.WriteFile(filepath.Join(dir, pkg, name), data, 0o644); err != nil {
				t.Fatal(err)
			}
		}
	}
	t.Chdir(dir)
	for _, scenario := range []struct {
		name     string
		patterns []string
		output   string
	}{
		{name: "committed dependency", patterns: []string{"./consumer"}},
		{name: "both selected", patterns: []string{"./consumer", "./dependency"}},
		{name: "missing generated files", patterns: []string{"./..."}, output: "missing"},
		{name: "stale generated files", patterns: []string{"./..."}, output: "stale"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			for _, pkg := range []string{"dependency", "consumer"} {
				path := filepath.Join(pkg, "contract_gen.go")
				if scenario.output == "missing" {
					if err := os.Remove(path); err != nil {
						t.Fatal(err)
					}
				}
				if scenario.output == "stale" {
					if err := os.WriteFile(
						path,
						[]byte("package "+pkg+"\nfunc (v Removed) Validate() error { return nil }\n"),
						0o644,
					); err != nil {
						t.Fatal(err)
					}
				}
			}
			if scenario.output == "missing" || scenario.output == "stale" {
				err := generate(t.Context(), scenario.patterns, false, "", true)
				if scenario.output == "missing" && !errors.Is(err, os.ErrNotExist) {
					t.Fatalf("check did not report missing output: %v", err)
				}
				if scenario.output == "stale" &&
					(err == nil || !strings.Contains(err.Error(), "generated file is stale")) {
					t.Fatalf("check did not report stale output: %v", err)
				}
				for _, pkg := range []string{"dependency", "consumer"} {
					data, err := os.ReadFile(filepath.Join(pkg, "contract_gen.go"))
					if scenario.output == "missing" && !errors.Is(err, os.ErrNotExist) {
						t.Fatalf("check wrote missing %s output: %v", pkg, err)
					}
					if scenario.output == "stale" && (err != nil || !strings.Contains(string(data), "Removed")) {
						t.Fatalf("check changed stale %s output: %v", pkg, err)
					}
				}
			}
			if err := generate(t.Context(), scenario.patterns, false, "", false); err != nil {
				t.Fatal(err)
			}
			if err := generate(t.Context(), scenario.patterns, false, "", true); err != nil {
				t.Fatal(err)
			}
			for _, pkg := range []string{"dependency", "consumer"} {
				got, err := os.ReadFile(filepath.Join(pkg, "contract_gen.go"))
				if err != nil {
					t.Fatal(err)
				}
				want, err := os.ReadFile(filepath.Join(fixture, pkg, "contract_gen.go"))
				if err != nil {
					t.Fatal(err)
				}
				if string(got) != string(want) {
					t.Fatalf("%s output differs after regeneration", pkg)
				}
			}
		})
	}
	// A declaration error must still fail even beside valid contract markers.
	if err := os.WriteFile(
		"consumer/broken.go",
		[]byte("package consumer\nvar Broken int = \"invalid\"\n"),
		0o644,
	); err != nil {
		t.Fatal(err)
	}
	if err := generate(
		t.Context(),
		[]string{"./consumer"},
		false,
		"",
		false,
	); err == nil ||
		!strings.Contains(err.Error(), "broken.go:") {
		t.Fatalf("declaration error was lost: %v", err)
	}
}

// Ordinary fixture code names every private generated entry point, so a naming
// regression also fails compilation. Local boundary call; budget <1 second.
func TestPrivateContractNames(t *testing.T) {
	input := []byte(`{"kind":"leaf","issuer":"local"}`)
	data, err := private.RoundTrip(input)
	if err != nil || string(data) != string(input) {
		t.Fatalf("private contract round trip: %s: %v", data, err)
	}
	file, err := parser.ParseFile(token.NewFileSet(), "testdata/private/contract_gen.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	for _, decl := range file.Decls {
		if fn, ok := decl.(*ast.FuncDecl); ok && fn.Recv == nil && fn.Name.IsExported() {
			t.Errorf("private contract exposed %s", fn.Name.Name)
		}
		if general, ok := decl.(*ast.GenDecl); ok && general.Tok == token.TYPE {
			for _, spec := range general.Specs {
				if typ := spec.(*ast.TypeSpec); typ.Name.IsExported() {
					t.Errorf("private holder exposed %s", typ.Name.Name)
				}
			}
		}
	}
}

// Scalar tables must retain the established TypeScript export spelling and
// literal form. Local generation and source read; budget one second.
func TestScalarTables(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/tables"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dest, "protocol", "tables.ts"))
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range []string{
		"export const MAX_PAGE_MESSAGE_BYTES = 1048576\n",
		"export const TABLE_LABEL = \"limits\"\n",
		"export const TABLE_ENABLED = true\n",
		"export const TABLE_RATIO = 0.5\n",
	} {
		if !strings.Contains(string(data), line) {
			t.Errorf("missing scalar table export: %s", line)
		}
	}
}

// The shape refusal names the unsafe field, not just its containing type.
// Cost: one local package load per direction, below one second total.
func TestUnsafeIntegerDiagnosticNamesField(t *testing.T) {
	for _, tc := range []struct {
		fixture   string
		direction string
	}{
		{"unsafe_integer", "maximum"},
		{"unsafe_integer_minimum", "minimum"},
	} {
		t.Run(tc.direction, func(t *testing.T) {
			err := generate(t.Context(), []string{"./testdata/invalid/" + tc.fixture}, false, "", false)
			if err == nil {
				t.Fatal("unsafe integer accepted")
			}
			for _, want := range []string{"types.go:4:6: Broken.Value:", "safe " + tc.direction} {
				if !strings.Contains(err.Error(), want) {
					t.Fatalf("unsafe integer diagnostic missing %q: %v", want, err)
				}
			}
		})
	}
}

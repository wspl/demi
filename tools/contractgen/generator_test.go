package main

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
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
	if err := generate(t.Context(), []string{"./testdata/blocks", "./testdata/runner", "./testdata/features", "./testdata/text", "./testdata/tables", "./testdata/kept", "./testdata/schemas", "./testdata/generics", "./testdata/remaining", "./testdata/keyed", "./testdata/browser", "./testdata/reachability", "./testdata/packedonly", "./testdata/schemacheck"}, false, "", true); err != nil {
		t.Fatal(err)
	}
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/blocks", "./testdata/features"}, true, dest, false); err != nil {
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
		if err := os.Mkdir(filepath.Join(dir, pkg), 0755); err != nil {
			t.Fatal(err)
		}
		for _, name := range []string{"types.go", "contract_gen.go"} {
			data, err := os.ReadFile(filepath.Join(fixture, pkg, name))
			if err != nil {
				t.Fatal(err)
			}
			data = []byte(strings.ReplaceAll(string(data), "testdata/loading/dependency", "testdata/"+filepath.Base(dir)+"/dependency"))
			if err := os.WriteFile(filepath.Join(dir, pkg, name), data, 0644); err != nil {
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
					if err := os.WriteFile(path, []byte("package "+pkg+"\nfunc (v Removed) Validate() error { return nil }\n"), 0644); err != nil {
						t.Fatal(err)
					}
				}
			}
			if scenario.output == "missing" || scenario.output == "stale" {
				err := generate(t.Context(), scenario.patterns, false, "", true)
				if scenario.output == "missing" && !errors.Is(err, os.ErrNotExist) {
					t.Fatalf("check did not report missing output: %v", err)
				}
				if scenario.output == "stale" && (err == nil || !strings.Contains(err.Error(), "generated file is stale")) {
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
	if err := os.WriteFile("consumer/broken.go", []byte("package consumer\nvar Broken int = \"invalid\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := generate(t.Context(), []string{"./consumer"}, false, "", false); err == nil || !strings.Contains(err.Error(), "broken.go:") {
		t.Fatalf("declaration error was lost: %v", err)
	}
}

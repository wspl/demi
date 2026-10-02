package main

import (
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
	if err := generate(t.Context(), []string{"./testdata/blocks", "./testdata/runner", "./testdata/features", "./testdata/text", "./testdata/tables", "./testdata/kept", "./testdata/schemas", "./testdata/generics", "./testdata/remaining", "./testdata/keyed", "./testdata/browser", "./testdata/reachability", "./testdata/packedonly"}, false, "", true); err != nil {
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

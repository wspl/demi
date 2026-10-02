package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/tables"
)

// This executes the generated TypeScript consumers, including formatting rules.
// Local package loading and Node cost below one second; npm ci in testdata supplies
// the pinned Zod dependency. The test never installs packages or uses the network.
func TestTableAndTextTypeScript(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/tables", "./testdata/text"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	expected, err := json.Marshal(map[string]any{"preview": tables.PREVIEW_TYPES, "models": tables.ModelFileTypes, "frames": tables.LiveViewFrameConstants})
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dest, "expected.json")
	if err := os.WriteFile(path, expected, 0600); err != nil {
		t.Fatal(err)
	}
	args := []string{"testdata/compare-rest.mjs", dest, path}
	if reference := os.Getenv("CONTRACTGEN_TABLE_REFERENCE"); reference != "" {
		args = append(args, reference)
	}
	cmd := exec.CommandContext(t.Context(), "node", args...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated TypeScript: %v\n%s", err, output)
	}
	t.Log(string(output))
}

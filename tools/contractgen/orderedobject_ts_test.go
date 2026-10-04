//go:build acceptance

package main

import (
	"os/exec"
	"testing"
)

// Checks generated Zod with pinned testdata npm dependencies. One local package
// load and Node process; budget 2 seconds, no network or services.
func TestOrderedObjectZod(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/orderedobject"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	cmd := exec.CommandContext(t.Context(), "node", "testdata/orderedobject/verify.mjs", dest)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("object Zod: %v\n%s", err, output)
	}
	t.Log(string(output))
}

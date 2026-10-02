//go:build acceptance

package main

import (
	"os/exec"
	"testing"
)

// Actual Zod must accept values refused only by the Go check while enforcing
// structural rules. Pinned testdata npm dependencies; budget one second, no network.
func TestSchemaGoOnlyChecksTypeScript(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/schemacheck"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	cmd := exec.CommandContext(t.Context(), "node", "testdata/schemacheck/verify.mjs", dest)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("Go-only check Zod: %v\n%s", err, output)
	}
	t.Log(string(output))
}

// Exercises actual Zod for both optional-null forms and the imported install
// limits. Pinned testdata npm dependencies; budget one second, no network.
func TestNullableOptionalTypeScript(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/presence"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	cmd := exec.CommandContext(t.Context(), "node", "testdata/presence/verify.mjs", dest)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("optional-null Zod: %v\n%s", err, output)
	}
	t.Log(string(output))
}

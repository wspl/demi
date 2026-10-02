//go:build acceptance

package main

import (
	"os/exec"
	"testing"
)

// Runs actual generated Zod against browser examples. Requires the pinned npm
// testdata dependencies; budget one second, no network, services, or timers.
func TestBrowserTypeScript(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/browser"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	cmd := exec.CommandContext(t.Context(), "node", "testdata/browser/verify.mjs", dest)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("browser Zod: %v\n%s", err, output)
	}
	t.Log(string(output))
}

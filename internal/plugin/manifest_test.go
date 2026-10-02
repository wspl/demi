package plugin

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

// TestRustManifests protects the complete plugin registration wire; no IO
// beyond one fixture read, subprocesses, network or wall-clock waits.
func TestRustManifests(t *testing.T) {
	source, err := os.ReadFile("testdata/manifests.json")
	if err != nil {
		t.Fatal(err)
	}
	values, err := decodeManifests(source)
	if err != nil {
		t.Fatal(err)
	}
	wire, err := values.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	var formatted bytes.Buffer
	if err := json.Indent(&formatted, wire, "", "  "); err != nil {
		t.Fatal(err)
	}
	formatted.WriteByte('\n')
	if !bytes.Equal(formatted.Bytes(), source) {
		t.Fatalf("Rust manifest round trip differs\n%s", formatted.Bytes())
	}
}

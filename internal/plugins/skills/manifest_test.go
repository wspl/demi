package skills

import (
	"bytes"
	"os"
	"testing"
)

// The host consumes these registration bytes; budget below one second.
func TestRustManifest(t *testing.T) {
	factory, err := New()
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	got, err := factory.Manifest().MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, bytes.TrimSpace(want)) {
		t.Fatalf("manifest differs from Rust:\ngot %s\nwant %s", got, want)
	}
}

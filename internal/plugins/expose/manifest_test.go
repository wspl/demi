package expose_test

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"

	"github.com/wspl/demi/internal/plugins/expose"
)

// TestManifestMatchesGolden compares the public factory's complete wire manifest
// with testdata/manifest.json, including schema annotations and order. Cost <1 s.
func TestManifestMatchesGolden(t *testing.T) {
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	var expected bytes.Buffer
	if err := json.Compact(&expected, data); err != nil {
		t.Fatal(err)
	}
	factory, err := expose.New()
	if err != nil {
		t.Fatal(err)
	}
	actual, err := factory.Manifest().MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(actual, expected.Bytes()) {
		t.Fatalf("expose manifest differs\ngot %s\nwant %s", actual, expected.Bytes())
	}
}

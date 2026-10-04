package file_test

import (
	"bytes"
	"os"
	"testing"

	"github.com/wspl/demi/internal/plugins/file"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// The registration bytes are read by the plugin host; budget below one second.
func TestManifestMatchesGolden(t *testing.T) {
	f, err := file.New()
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	got, err := f.Manifest().MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, bytes.TrimSpace(want)) {
		t.Fatalf("manifest differs from testdata/manifest.json:\ngot %s\nwant %s", got, want)
	}
}

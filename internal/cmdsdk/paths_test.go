package cmdsdk_test

import (
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
)

func TestResolveRefusesEmptyPathsAndKeepsSymlinkComponents(t *testing.T) {
	for _, path := range []string{"", "bad\x00path"} {
		if _, err := cmdsdk.Resolve("/work", path); err == nil {
			t.Fatalf("Resolve(%q) accepted an invalid path", path)
		}
	}
	got, err := cmdsdk.Resolve("/work", "link/../file")
	if err != nil {
		t.Fatal(err)
	}
	if want := "/work/link/../file"; got != want {
		t.Fatalf("Resolve = %q; want %q", got, want)
	}
}

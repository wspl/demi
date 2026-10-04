package commandsdk_test

import (
	"testing"

	"github.com/wspl/demi/internal/commandsdk"
)

func TestResolveRefusesEmptyPathsAndKeepsSymlinkComponents(t *testing.T) {
	for _, path := range []string{"", "bad\x00path"} {
		if _, err := commandsdk.Resolve("/work", path); err == nil {
			t.Fatalf("Resolve(%q) accepted an invalid path", path)
		}
	}
	got, err := commandsdk.Resolve("/work", "link/../file")
	if err != nil {
		t.Fatal(err)
	}
	if want := "/work/link/../file"; got != want {
		t.Fatalf("Resolve = %q; want %q", got, want)
	}
}

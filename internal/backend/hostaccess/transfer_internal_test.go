package hostaccess

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// A file's version derives from its size and modification time, and
// If-None-Match compares it weakly (docs/product/web-api.md, fs/raw).
// The version's spelling is the backend's own and is not pinned here.
// Cost: no IO, processes, or wall-clock waits.
func TestFileVersionsAndWeakConditions(t *testing.T) {
	at := func(millis int64) types.Timestamp {
		t.Helper()
		stamp, err := types.TimestampFromMillisecond(millis)
		if err != nil {
			t.Fatal(err)
		}
		return stamp
	}
	stat := host.FileStat{Kind: host.File, Mode: 0o644, Size: 300000, Modified: at(1790000000123)}
	version := FileVersion(stat)
	modeChanged := stat
	modeChanged.Mode = 0o600
	if got := FileVersion(modeChanged); got != version {
		t.Fatalf("mode change: version %s, want %s", got, version)
	}
	resized := stat
	resized.Size++
	touched := stat
	touched.Modified = at(1790000000124)
	for name, other := range map[string]host.FileStat{"resized": resized, "touched": touched} {
		if FileVersion(other) == version {
			t.Fatalf("%s file kept version %s", name, version)
		}
	}
	before := host.FileStat{Size: 16, Modified: at(-15)}
	after := host.FileStat{Size: 16, Modified: at(15)}
	if FileVersion(before) == FileVersion(after) {
		t.Fatalf("times before and after the epoch share version %s", FileVersion(before))
	}
	strong := strings.TrimPrefix(version, "W/")
	for _, condition := range []string{version, strong, `"other", ` + version, " * "} {
		if !notModified(condition, version) {
			t.Fatalf("condition %q did not match %s", condition, version)
		}
	}
	for _, condition := range []string{FileVersion(resized), `"other"`, ""} {
		if notModified(condition, version) {
			t.Fatalf("condition %q matched %s", condition, version)
		}
	}
}

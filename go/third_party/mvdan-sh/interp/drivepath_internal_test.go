package interp

import "testing"

// Drive paths convert as the Rust runner's resolve_path does; the conversion
// is checked on every system, while DrivePath applies it on Windows only.
func TestDrivePath(t *testing.T) {
	for path, want := range map[string]string{
		"/c":         "c:/",
		"/c/":        "c:/",
		"/D/Users/a": "D:/Users/a",
		"/dev/null":  "/dev/null",
		"/c\\x":      "/c\\x",
		"c/x":        "c/x",
		"/1/x":       "/1/x",
		"/":          "/",
		"":           "",
	} {
		if got := drivePath(path); got != want {
			t.Errorf("%q: %q, want %q", path, got, want)
		}
	}
}

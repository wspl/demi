package engine

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestWindowsDrivePathsWorkForCdRedirectionUtilitiesAndExecutables(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "sub"), 0700); err != nil {
		t.Fatal(err)
	}
	drive := func(path string) string {
		path = filepath.ToSlash(path)
		return "/" + path[:1] + path[2:]
	}
	// Rust also passes the drive-form path to cat; retain that utility boundary.
	result, output, stderr := shellFiles(t, root, `printf discarded > /dev/null && printf discarded &> /dev/null && printf payload > "$DRIVE_ROOT/file" && cd "$DRIVE_ROOT/sub" && cat "$DRIVE_ROOT/file" && "$DRIVE_EXE" /c echo external`, func(options *Options) {
		options.Env["DRIVE_ROOT"] = drive(root)
		options.Env["SystemRoot"] = os.Getenv("SystemRoot")
		options.Env["DRIVE_EXE"] = drive(filepath.Join(os.Getenv("SystemRoot"), "System32", "cmd.exe"))
	})
	if result.Code != 0 || !strings.EqualFold(result.Cwd, filepath.Join(root, "sub")) || output != "payloadexternal\r\n" {
		t.Fatalf("%+v %q %q", result, output, stderr)
	}
}

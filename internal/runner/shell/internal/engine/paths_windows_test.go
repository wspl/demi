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
	// System programs accept their native path syntax; unlike embedded utilities,
	// their arguments are not interpreted or rewritten by the runner.
	result, output, stderr := shellFiles(t, root, `printf discarded > /dev/null && printf discarded &> /dev/null && printf payload > "$DRIVE_ROOT/file" && cd "$DRIVE_ROOT/sub" && "$DRIVE_EXE" /c type "$NATIVE_ROOT\file" && "$DRIVE_EXE" /c echo external`, func(options *Options) {
		options.Env["DRIVE_ROOT"] = drive(root)
		options.Env["NATIVE_ROOT"] = root
		options.Env["SystemRoot"] = os.Getenv("SystemRoot")
		options.Env["DRIVE_EXE"] = drive(filepath.Join(os.Getenv("SystemRoot"), "System32", "cmd.exe"))
	})
	if result.Code != 0 || !strings.EqualFold(result.Cwd, filepath.Join(root, "sub")) || output != "payloadexternal\r\n" {
		t.Fatalf("%+v %q %q", result, output, stderr)
	}
}

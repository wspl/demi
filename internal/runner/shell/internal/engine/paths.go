package engine

import (
	"os"
	"runtime"
)

// shellPath translates the shell's device and drive syntax at OS boundaries.
func shellPath(path string) string {
	if runtime.GOOS != "windows" {
		return path
	}
	if path == "/dev/null" {
		return os.DevNull
	}
	if len(path) >= 3 && path[0] == '/' && path[2] == '/' &&
		((path[1] >= 'A' && path[1] <= 'Z') || (path[1] >= 'a' && path[1] <= 'z')) {
		return path[1:2] + ":" + path[2:]
	}
	return path
}

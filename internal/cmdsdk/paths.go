package cmdsdk

import (
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// Resolve resolves a path against invocation cwd without collapsing symlink-sensitive '..'.
func Resolve(cwd, path string) (string, error) {
	if path == "" || strings.ContainsRune(path, 0) {
		return "", errors.New("path must be nonempty and contain no NUL byte")
	}
	if filepath.IsAbs(path) {
		return path, nil
	}
	if runtime.GOOS == "windows" {
		if filepath.VolumeName(path) != "" {
			return path, nil
		}
		if path[0] == '\\' || path[0] == '/' {
			return filepath.VolumeName(cwd) + path, nil
		}
	}
	if cwd == "" {
		return path, nil
	}
	return strings.TrimRight(cwd, string(os.PathSeparator)) + string(os.PathSeparator) + path, nil
}

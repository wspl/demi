//go:build darwin || linux

package process

import (
	"errors"
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/sys/unix"
)

// lookupExecutable resolves the raw process against its own PATH and cwd.
// exec.LookPath reads the runner's environment, so it cannot perform this lookup.
func lookupExecutable(command, cwd string, env map[string]string) (string, error) {
	if strings.ContainsRune(command, '/') {
		return command, nil
	}
	path, ok := env["PATH"]
	if !ok {
		path = defaultExecutablePath
	} // execvp's Unix default when PATH is absent.
	var failure error
	for _, directory := range strings.Split(path, ":") {
		candidate := filepath.Join(directory, command)
		if !filepath.IsAbs(candidate) {
			candidate = filepath.Join(cwd, candidate)
		}
		info, err := os.Stat(candidate)
		if err == nil {
			if info.IsDir() {
				err = unix.EACCES
			} else {
				err = unix.Faccessat(unix.AT_FDCWD, candidate, unix.X_OK, unix.AT_EACCESS)
			}
		}
		if err == nil {
			return candidate, nil
		}
		if !errors.Is(err, os.ErrNotExist) && !errors.Is(err, unix.ENOTDIR) {
			failure = err
		}
	}
	if failure == nil {
		failure = unix.ENOENT
	}
	return "", &os.PathError{Op: "exec", Path: command, Err: failure}
}

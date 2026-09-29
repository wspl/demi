//go:build linux

package linux_test

import (
	"io/fs"
	"syscall"
)

// umask sets the process's umask and returns the one it replaced.
func umask(mask fs.FileMode) fs.FileMode {
	return fs.FileMode(syscall.Umask(int(mask)))
}

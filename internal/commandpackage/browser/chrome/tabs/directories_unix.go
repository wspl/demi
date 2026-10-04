//go:build unix

package tabs

import (
	"errors"
	"os"
	"syscall"

	"golang.org/x/sys/unix"
)

// lockEnvironment is nonblocking so a sweep never waits for a running browser.
// artifacts.AcquireInstallLock waits and creates absent files; that API cannot
// implement the orphan sweep's existing-file-only probe.
func lockEnvironment(file *os.File) (bool, error) {
	err := unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB)
	if errors.Is(err, unix.EWOULDBLOCK) {
		return false, nil
	}
	return err == nil, err
}

func currentOwner() uint32 {
	return uint32(os.Getuid())
}

// ownsDirectory excludes symbolic links and another user's browser storage.
func ownsDirectory(path string, owner uint32) bool {
	info, err := os.Lstat(path)
	if err != nil || !info.IsDir() {
		return false
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	return ok && stat.Uid == owner
}

func directoryNotEmpty(err error) bool {
	return errors.Is(err, unix.ENOTEMPTY)
}

// openEnvironmentLock never creates a missing lock during an orphan probe.
func openEnvironmentLock(path string, create bool) (*os.File, error) {
	flags := os.O_RDWR
	if create {
		flags |= os.O_CREATE | os.O_EXCL
	}
	return os.OpenFile(path, flags, 0o600)
}

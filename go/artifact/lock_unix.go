//go:build unix

package artifact

import (
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

// tryLock tries to take an exclusive lock on file without waiting. It reports
// whether another holds the lock. The lock belongs to the open file, so a second
// open of the same file in one process finds it held.
func tryLock(file *os.File) (held bool, err error) {
	for {
		err := unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB)
		switch {
		case err == nil:
			return false, nil
		case errors.Is(err, unix.EINTR):
			continue
		case errors.Is(err, unix.EWOULDBLOCK):
			return true, nil
		default:
			return false, os.NewSyscallError("flock", err)
		}
	}
}

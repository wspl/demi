//go:build unix

package filelock

import (
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

func lock(file *os.File) error {
	for {
		err := unix.Flock(int(file.Fd()), unix.LOCK_EX)
		switch {
		case err == nil:
			return nil
		case errors.Is(err, unix.EINTR):
			continue
		default:
			return os.NewSyscallError("flock", err)
		}
	}
}

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

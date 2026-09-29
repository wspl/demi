package filelock

import (
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

func lock(file *os.File) error {
	overlapped := new(windows.Overlapped)
	if err := windows.LockFileEx(windows.Handle(file.Fd()), windows.LOCKFILE_EXCLUSIVE_LOCK, 0, 1, 0, overlapped); err != nil {
		return os.NewSyscallError("LockFileEx", err)
	}
	return nil
}

func tryLock(file *os.File) (held bool, err error) {
	overlapped := new(windows.Overlapped)
	err = windows.LockFileEx(windows.Handle(file.Fd()), windows.LOCKFILE_EXCLUSIVE_LOCK|windows.LOCKFILE_FAIL_IMMEDIATELY, 0, 1, 0, overlapped)
	switch {
	case err == nil:
		return false, nil
	case errors.Is(err, windows.ERROR_LOCK_VIOLATION):
		return true, nil
	default:
		return false, os.NewSyscallError("LockFileEx", err)
	}
}

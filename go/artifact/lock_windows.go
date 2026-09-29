package artifact

import (
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

// tryLock tries to take an exclusive lock on file without waiting. It reports
// whether another holds the lock. The lock belongs to the open file, so a second
// open of the same file in one process finds it held.
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

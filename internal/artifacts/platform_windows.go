package artifacts

import (
	"context"
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

func tryLock(file *os.File) (bool, error) {
	err := windows.LockFileEx(windows.Handle(file.Fd()), windows.LOCKFILE_EXCLUSIVE_LOCK|windows.LOCKFILE_FAIL_IMMEDIATELY, 0, 1, 0, &windows.Overlapped{})
	if errors.Is(err, windows.ERROR_LOCK_VIOLATION) {
		return false, nil
	}
	return err == nil, err
}

// Windows has no portable directory fsync; as in Rust, the system persists entries.
func syncDirectory(ctx context.Context, _ string) error { return ctx.Err() }

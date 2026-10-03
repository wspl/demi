package tabs

import (
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

func lockEnvironment(file *os.File) (bool, error) {
	err := windows.LockFileEx(
		windows.Handle(file.Fd()),
		windows.LOCKFILE_EXCLUSIVE_LOCK|windows.LOCKFILE_FAIL_IMMEDIATELY,
		0,
		1,
		0,
		&windows.Overlapped{},
	)
	if errors.Is(err, windows.ERROR_LOCK_VIOLATION) {
		return false, nil
	}
	return err == nil, err
}

func currentOwner() uint32 { return 0 }

// Windows browser storage is below the user's private temporary directory.
func ownsDirectory(path string, _ uint32) bool {
	info, err := os.Lstat(path)
	return err == nil && info.IsDir()
}

func directoryNotEmpty(err error) bool { return errors.Is(err, windows.ERROR_DIR_NOT_EMPTY) }

// openEnvironmentLock permits rename/removal while the lock is still held, as
// Rust's OpenOptions does. os.OpenFile omits FILE_SHARE_DELETE on Windows.
func openEnvironmentLock(path string, create bool) (*os.File, error) {
	name, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return nil, err
	}
	disposition := uint32(windows.OPEN_EXISTING)
	if create {
		disposition = windows.CREATE_NEW
	}
	handle, err := windows.CreateFile(
		name,
		windows.GENERIC_READ|windows.GENERIC_WRITE,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE|windows.FILE_SHARE_DELETE,
		nil,
		disposition,
		windows.FILE_ATTRIBUTE_NORMAL,
		0,
	)
	if err != nil {
		return nil, &os.PathError{Op: "open", Path: path, Err: err}
	}
	return os.NewFile(uintptr(handle), path), nil
}

//go:build linux

package machines

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/unix"
)

// inheritedDescriptor is the descriptor the recovery process receives the state
// lock on: the first of the descriptors a child is given beside its standard
// three.
const inheritedDescriptor = 3

// An OwnedError means another manager owns a directory.
type OwnedError struct {
	Path string
}

func (e *OwnedError) Error() string { return "Another Cloud manager owns " + e.Path }

// A NotInheritedError means the recovery process was started without the
// manager's lock.
type NotInheritedError struct {
	Path string
}

func (e *NotInheritedError) Error() string {
	return "the recovery process needs the manager's lock on " + e.Path
}

// A ManagerLock holds the manager's two exclusive locks (docs/cloud/managed-hosts.md
// § Control and ownership): one on its state directory, one on its runtime
// directory, so a second manager on either is refused. The kernel releases them
// when the process ends, however it ends.
type ManagerLock struct {
	data    *os.File
	runtime *os.File
}

func lockFile(directory string) string {
	return filepath.Join(directory, "manager.lock")
}

func takeLock(path string) (*os.File, error) {
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o600)
	if err != nil {
		return nil, err
	}
	err = unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB)
	switch {
	case err == nil:
		return file, nil
	case errors.Is(err, unix.EWOULDBLOCK):
		file.Close()
		return nil, &OwnedError{Path: path}
	}
	file.Close()
	return nil, fmt.Errorf("locking %s: %w", path, err)
}

// AcquireLocks takes both locks.
func AcquireLocks(data, runtime string) (*ManagerLock, error) {
	dataFile, err := takeLock(lockFile(data))
	if err != nil {
		return nil, err
	}
	runtimeFile, err := takeLock(lockFile(runtime))
	if err != nil {
		dataFile.Close()
		return nil, err
	}
	return &ManagerLock{data: dataFile, runtime: runtimeFile}, nil
}

// Close releases both locks.
func (l *ManagerLock) Close() error {
	return errors.Join(l.data.Close(), l.runtime.Close())
}

// VerifyInherited checks that the descriptor the recovery process inherited is
// this state directory's lock file and that this process may hold the lock: it
// shares the lock of the manager that started it, or no manager holds it. A
// process started by hand beside a running manager has neither, so it cannot
// recover live devices.
func VerifyInherited(data string) error {
	path := lockFile(data)
	refused := &NotInheritedError{Path: path}
	var held, expected unix.Stat_t
	if err := unix.Fstat(inheritedDescriptor, &held); err != nil {
		return refused
	}
	if err := unix.Stat(path, &expected); err != nil {
		return refused
	}
	if held.Dev != expected.Dev || held.Ino != expected.Ino {
		return refused
	}
	if err := unix.Flock(inheritedDescriptor, unix.LOCK_EX|unix.LOCK_NB); err != nil {
		return refused
	}
	return nil
}

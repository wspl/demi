//go:build linux

package machinemanager

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/machinemanager/sandbox"
	"golang.org/x/sys/unix"
)

// ManagerLock holds exclusive state and runtime locks until Close.
type ManagerLock struct{ data, runtime *os.File }

// AcquireLock refuses another manager on either directory.
func AcquireLock(data, runtime string) (*ManagerLock, error) {
	d, err := takeLock(filepath.Join(data, "manager.lock"))
	if err != nil {
		return nil, err
	}
	r, err := takeLock(filepath.Join(runtime, "manager.lock"))
	if err != nil {
		// No bytes were written; closing only releases the acquired lock.
		_ = d.Close()
		return nil, err
	}
	return &ManagerLock{data: d, runtime: r}, nil
}

// takeLock opens and exclusively locks one manager directory's lock file.
func takeLock(path string) (*os.File, error) {
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return nil, err
	}
	if err = unix.Flock(int(f.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		// This descriptor has no buffered writes and acquired no lock.
		_ = f.Close()
		if errors.Is(err, unix.EWOULDBLOCK) {
			//nolint:staticcheck // User-visible text, kept byte for byte.
			return nil, fmt.Errorf("Another Cloud manager owns %s", path)
		}
		return nil, fmt.Errorf("locking %s: %w", path, err)
	}
	return f, nil
}

// Data returns the borrowed state lock inherited by the recovery process.
func (l *ManagerLock) Data() *os.File { return l.data }

// Close releases both locks.
func (l *ManagerLock) Close() error { return errors.Join(l.runtime.Close(), l.data.Close()) }

// VerifyInherited checks descriptor 3 names and holds this state directory's lock.
func VerifyInherited(data string) error {
	path := filepath.Join(data, "manager.lock")
	var held, expected unix.Stat_t
	if unix.Fstat(sandbox.RecoveryLockFD, &held) != nil || unix.Stat(path, &expected) != nil ||
		held.Dev != expected.Dev ||
		held.Ino != expected.Ino ||
		unix.Flock(sandbox.RecoveryLockFD, unix.LOCK_EX|unix.LOCK_NB) != nil {
		return fmt.Errorf("the recovery process needs the manager's lock on %s", path)
	}
	return nil
}

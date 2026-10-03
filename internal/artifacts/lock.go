package artifacts

import (
	"context"
	"errors"
	"os"
	"sync/atomic"
	"time"
)

var lockWaits atomic.Uint64

// InstallLock owns an OS file lock shared by installers in every process.
// Defer Close after acquisition. Do not copy a lock or close it concurrently.
type InstallLock struct{ file *os.File }

// AcquireInstallLock waits cancellably for an exclusive installation lock.
func AcquireInstallLock(ctx context.Context, path string) (*InstallLock, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o666)
	if err != nil {
		return nil, err
	}
	waiting := false
	for {
		if err := ctx.Err(); err != nil {
			return nil, errors.Join(err, f.Close())
		}
		held, err := tryLock(f)
		if err != nil {
			return nil, errors.Join(err, f.Close())
		}
		if held {
			return &InstallLock{f}, nil
		}
		if !waiting {
			lockWaits.Add(1)
			waiting = true
		}
		timer := time.NewTimer(50 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil, errors.Join(ctx.Err(), f.Close())
		case <-timer.C:
			timer.Stop()
		}
	}
}

// Close releases the installation lock and its file. It is idempotent.
func (l *InstallLock) Close() error {
	if l.file == nil {
		return nil
	}
	err := l.file.Close()
	l.file = nil
	return err
}

// InstallLockWaits is the process-wide count of acquisitions that waited.
// It is exposed for artifactstest; product code has no use for this observation.
func InstallLockWaits() uint64 { return lockWaits.Load() }

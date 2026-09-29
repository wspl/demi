package artifact

import (
	"context"
	"os"
	"time"
)

// lockRetry is how often a waiting installer tries the lock again.
const lockRetry = 50 * time.Millisecond

// An InstallLock is an exclusive OS lock on a file beside an installation, held
// until [InstallLock.Close]. Every process installing the same artifact takes it
// first, so one downloads and the others find its result.
type InstallLock struct {
	// file holds the lock: closing it releases the lock.
	file *os.File
}

// lockWaitsKey is the context key of the function that hears of a wait.
type lockWaitsKey struct{}

// ObserveLockWaits returns a context in which every acquisition of an install
// lock that finds the lock held calls observe once. Nothing else shows that an
// installer waits for another instead of installing beside it, so a test of
// concurrent installers counts the waits through it.
func ObserveLockWaits(ctx context.Context, observe func()) context.Context {
	return context.WithValue(ctx, lockWaitsKey{}, observe)
}

// AcquireInstallLock waits for the lock on path, creating the file; cancelling
// ctx gives up the wait.
func AcquireInstallLock(ctx context.Context, path string) (*InstallLock, error) {
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o666)
	if err != nil {
		return nil, err
	}
	waiting := false
	for {
		held, err := tryLock(file)
		if err != nil {
			file.Close()
			return nil, err
		}
		if !held {
			return &InstallLock{file: file}, nil
		}
		if !waiting {
			waiting = true
			if observe, ok := ctx.Value(lockWaitsKey{}).(func()); ok {
				observe()
			}
		}
		retry := time.NewTimer(lockRetry)
		select {
		case <-ctx.Done():
			retry.Stop()
			file.Close()
			return nil, ctx.Err()
		case <-retry.C:
		}
	}
}

// Close releases the lock.
func (l *InstallLock) Close() error {
	return l.file.Close()
}

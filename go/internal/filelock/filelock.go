// Package filelock takes exclusive locks on open files, which the operating
// system gives up when the file is closed or its process ends. A lock is
// advisory: it holds off other holders of the same lock, and only those.
package filelock

import "os"

// Lock waits until it holds an exclusive lock on file. The lock belongs to the
// open file, so a second open of the same file, in this process or another,
// waits for it. Closing file releases it.
func Lock(file *os.File) error {
	return lock(file)
}

// TryLock takes an exclusive lock on file without waiting. It reports whether
// another holds the lock; then it holds nothing.
func TryLock(file *os.File) (held bool, err error) {
	return tryLock(file)
}

//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// CloneSparse copies an image using a reflink when available, otherwise copying
// data ranges while preserving holes and skipping zero blocks. It creates a
// new destination with mode 0600, preserving the source length and bytes.
func CloneSparse(ctx context.Context, source, destination string) error {
	panic("not written: m-storage")
}

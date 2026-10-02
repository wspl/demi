//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// CopySkeleton copies skeleton to a new destination, preserving symbolic links
// and modes and assigning every entry to the sandbox user.
func CopySkeleton(ctx context.Context, skeleton, destination string) error {
	panic("not written: m-storage")
}

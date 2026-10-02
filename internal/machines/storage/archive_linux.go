//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// VetArchive checks every entry of a zstd-compressed tar archive before extraction.
func VetArchive(ctx context.Context, archive string) error { panic("not written: m-storage") }

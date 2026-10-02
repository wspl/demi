//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// WriteJSON replaces path with value's JSON, synced before rename. The caller
// syncs its directory when the rename must survive a crash. Values are contract
// types or ordered structs, never unordered object maps.
func WriteJSON(ctx context.Context, path string, value any) error { panic("not written: m-storage") }

// Sync syncs a file's data or a directory's entries to disk.
func Sync(ctx context.Context, path string) error { panic("not written: m-storage") }

// RemoveTree removes a directory tree; one already gone is fine.
func RemoveTree(ctx context.Context, path string) error { panic("not written: m-storage") }

// CreatePrivate creates path and missing parents private to the owner.
// Existing directories retain their modes.
func CreatePrivate(ctx context.Context, path string) error { panic("not written: m-storage") }

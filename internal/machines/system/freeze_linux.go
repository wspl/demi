//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// ThawResult describes what a thaw found.
type ThawResult uint8

const (
	// Thawed means the filesystem was thawed.
	Thawed ThawResult = iota
	// NotFrozen means the filesystem was not frozen, as after a failed freeze or an earlier thaw.
	NotFrozen
)

// Freeze freezes the filesystem mounted at mount.
// Once issued, the syscall completes even if ctx is canceled.
func Freeze(ctx context.Context, mount string) error { panic("not written: m-system") }

// Thaw thaws the filesystem mounted at mount.
// Cleanup callers use a context without cancellation so every recorded mount is thawed.
func Thaw(ctx context.Context, mount string) (ThawResult, error) { panic("not written: m-system") }

// Frozen records filesystems frozen for one checkpoint. The zero value is ready
// to use. Its owner defers ThawAll before the first Freeze, using a context
// without cancellation, and handles the returned failures. It is not concurrent.
type Frozen struct{}

// NewFrozen creates an empty checkpoint freeze guard.
func NewFrozen() *Frozen { panic("not written: m-system") }

// Freeze records mount before issuing the freeze, since a failed freeze may
// still have frozen the filesystem. The frozen window must not be canceled.
func (f *Frozen) Freeze(ctx context.Context, mount string) error { panic("not written: m-system") }

// ThawAll thaws every recorded filesystem and returns what failed. It drains
// the records, so a deferred second call has nothing to thaw.
func (f *Frozen) ThawAll(ctx context.Context) []error { panic("not written: m-system") }

//go:build linux

package system

import (
	"context"
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

// Linux UAPI linux/fs.h: _IOWR('X', 119/120, int), identical on the
// supported amd64 and arm64 targets. x/sys/unix does not expose these names.
const (
	fiFreeze = 0xc0045877
	fiThaw   = 0xc0045878
)

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
func Freeze(ctx context.Context, mount string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	directory, err := os.Open(mount)
	if err != nil {
		return err
	}
	// Closing this read-only descriptor has no buffered writes to report.
	defer func() { _ = directory.Close() }()
	return Failed("freezing", mount, unix.IoctlSetInt(int(directory.Fd()), fiFreeze, 0))
}

// Thaw thaws the filesystem mounted at mount.
// Cleanup callers use a context without cancellation so every recorded mount is thawed.
func Thaw(ctx context.Context, mount string) (ThawResult, error) {
	if err := ctx.Err(); err != nil {
		return NotFrozen, err
	}
	directory, err := os.Open(mount)
	if err != nil {
		return NotFrozen, err
	}
	// Closing this read-only descriptor has no buffered writes to report.
	defer func() { _ = directory.Close() }()
	err = unix.IoctlSetInt(int(directory.Fd()), fiThaw, 0)
	if errors.Is(err, unix.EINVAL) {
		return NotFrozen, nil
	}
	if err != nil {
		return NotFrozen, Failed("thawing", mount, err)
	}
	return Thawed, nil
}

// Frozen records filesystems frozen for one checkpoint. The zero value is ready
// to use. Its owner defers ThawAll before the first Freeze, using a context
// without cancellation, and handles the returned failures. It is not concurrent.
type Frozen struct{ mounts []string }

// NewFrozen creates an empty checkpoint freeze guard.
func NewFrozen() *Frozen { return &Frozen{} }

// Freeze records mount before issuing the freeze, since a failed freeze may
// still have frozen the filesystem. The frozen window must not be canceled.
func (f *Frozen) Freeze(ctx context.Context, mount string) error {
	f.mounts = append(f.mounts, mount)
	return Freeze(ctx, mount)
}

// ThawAll thaws every recorded filesystem and returns what failed. It drains
// the records, so a deferred second call has nothing to thaw.
func (f *Frozen) ThawAll(ctx context.Context) []error {
	var failures []error
	mounts := f.mounts
	f.mounts = nil
	for _, mount := range mounts {
		if _, err := Thaw(ctx, mount); err != nil {
			failures = append(failures, err)
		}
	}
	return failures
}

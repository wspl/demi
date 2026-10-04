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

// Thaw thaws the filesystem mounted at mount and reports whether it was frozen.
// Cleanup callers use a context without cancellation so every recorded mount is thawed.
func Thaw(ctx context.Context, mount string) (thawed bool, err error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	directory, err := os.Open(mount)
	if err != nil {
		return false, err
	}
	// Closing this read-only descriptor has no buffered writes to report.
	defer func() { _ = directory.Close() }()
	err = unix.IoctlSetInt(int(directory.Fd()), fiThaw, 0)
	if errors.Is(err, unix.EINVAL) {
		return false, nil
	}
	if err != nil {
		return false, Failed("thawing", mount, err)
	}
	return true, nil
}

// Frozen records filesystems frozen for one checkpoint. The zero value is ready
// to use. Its owner defers ThawAll before the first Freeze, using a context
// without cancellation, and handles the returned failures. It is not concurrent.
type Frozen struct{ mounts []string }

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

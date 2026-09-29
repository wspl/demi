//go:build linux

package linux

import (
	"errors"
	"log/slog"
	"os"

	"golang.org/x/sys/unix"
)

// The freeze ioctls, _IOWR('X', 119, int) and _IOWR('X', 120, int): x/sys has
// no names for them.
const (
	fifreeze = 0xc0045877
	fithaw   = 0xc0045878
)

// Freeze flushes the filesystem mounted at mount and blocks its writers.
func Freeze(mount string) error {
	directory, err := os.Open(mount)
	if err != nil {
		return err
	}
	defer directory.Close()
	if err := unix.IoctlSetInt(int(directory.Fd()), fifreeze, 0); err != nil {
		return Failed("freezing", mount, err)
	}
	return nil
}

// Thaw releases the filesystem mounted at mount. It reports false when the
// filesystem was not frozen, as after a freeze that failed or a thaw that ran
// already.
func Thaw(mount string) (thawed bool, err error) {
	directory, err := os.Open(mount)
	if err != nil {
		return false, err
	}
	defer directory.Close()
	err = unix.IoctlSetInt(int(directory.Fd()), fithaw, 0)
	switch {
	case err == nil:
		return true, nil
	case errors.Is(err, unix.EINVAL):
		return false, nil
	}
	return false, Failed("thawing", mount, err)
}

// Frozen is the set of filesystems frozen for one checkpoint. Each freeze is
// recorded before it is issued, because a failed one may still have frozen the
// filesystem; [Frozen.ThawAll] thaws them all, and a deferred [Frozen.Release]
// thaws what is left when the job returns or panics.
type Frozen struct {
	mounts []string
}

// Freeze records mount and freezes it.
func (f *Frozen) Freeze(mount string) error {
	f.mounts = append(f.mounts, mount)
	return Freeze(mount)
}

// ThawAll thaws every recorded filesystem and returns what failed.
func (f *Frozen) ThawAll() []error {
	var failed []error
	for _, mount := range f.mounts {
		if _, err := Thaw(mount); err != nil {
			failed = append(failed, err)
		}
	}
	f.mounts = nil
	return failed
}

// Release thaws what is still frozen and logs what fails. It is the deferred
// half of the guard, for the paths on which the job did not thaw itself.
func (f *Frozen) Release() {
	for _, err := range f.ThawAll() {
		slog.Error("machines: " + err.Error())
	}
}

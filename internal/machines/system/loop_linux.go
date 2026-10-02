//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// LoopDevice is an attached loop device held open until its filesystem is
// mounted. The owner defers Close; closing before a mount holds it detaches it.
type LoopDevice struct{}

// Attach attaches image to a free loop device with auto-clear set.
func Attach(ctx context.Context, image string) (*LoopDevice, error) { panic("not written: m-system") }

// Number returns the attached device's number, retained for growth.
func (d *LoopDevice) Number() uint32 { panic("not written: m-system") }

// Path returns the attached device's /dev/loop<number> path.
func (d *LoopDevice) Path() string { panic("not written: m-system") }

// Close releases the device descriptor. It is idempotent; a mounted filesystem
// keeps the device attached until unmount.
func (d *LoopDevice) Close() error { panic("not written: m-system") }

// LoopPath returns /dev/loop<number>.
func LoopPath(number uint32) string { panic("not written: m-system") }

// RefreshCapacity makes the loop device see its backing file's new size.
func RefreshCapacity(ctx context.Context, number uint32) error { panic("not written: m-system") }

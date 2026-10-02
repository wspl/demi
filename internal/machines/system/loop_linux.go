//go:build linux

package system

import (
	"context"
	"errors"
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// LoopDevice is an attached loop device held open until its filesystem is
// mounted. The owner defers Close; closing before a mount holds it detaches it.
type LoopDevice struct {
	number uint32
	device *os.File
}

// Attach attaches image to a free loop device with auto-clear set.
func Attach(ctx context.Context, image string) (*LoopDevice, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	backing, err := os.OpenFile(image, os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	// No writes are made through these descriptors; the loop driver holds its
	// own reference to the backing file after successful configuration.
	defer func() { _ = backing.Close() }()
	control, err := os.OpenFile("/dev/loop-control", os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	defer func() { _ = control.Close() }()
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		number, err := unix.IoctlRetInt(int(control.Fd()), unix.LOOP_CTL_GET_FREE)
		if err != nil {
			return nil, Failed("finding a free loop device through", "/dev/loop-control", err)
		}
		path := LoopPath(uint32(number))
		device, err := os.OpenFile(path, os.O_RDWR, 0)
		if err != nil {
			return nil, err
		}
		config := unix.LoopConfig{Fd: uint32(backing.Fd()), Info: unix.LoopInfo64{Flags: unix.LO_FLAGS_AUTOCLEAR}}
		err = unix.IoctlLoopConfigure(int(device.Fd()), &config)
		if err == nil {
			return &LoopDevice{number: uint32(number), device: device}, nil
		}
		// Configuration failed; no writes or live attachment belong to this fd.
		_ = device.Close()
		if errors.Is(err, unix.EBUSY) {
			continue
		}
		return nil, Failed("attaching", path, err)
	}
}

// Number returns the attached device's number, retained for growth.
func (d *LoopDevice) Number() uint32 { return d.number }

// Path returns the attached device's /dev/loop<number> path.
func (d *LoopDevice) Path() string { return LoopPath(d.number) }

// Close releases the device descriptor. It is idempotent; a mounted filesystem
// keeps the device attached until unmount.
func (d *LoopDevice) Close() error {
	if d.device == nil {
		return nil
	}
	device := d.device
	d.device = nil
	return device.Close()
}

// LoopPath returns /dev/loop<number>.
func LoopPath(number uint32) string { return fmt.Sprintf("/dev/loop%d", number) }

// RefreshCapacity makes the loop device see its backing file's new size.
func RefreshCapacity(ctx context.Context, number uint32) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	path := LoopPath(number)
	device, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	// The ioctl does not buffer writes on this descriptor.
	defer func() { _ = device.Close() }()
	return Failed("refreshing the capacity of", path, unix.IoctlSetInt(int(device.Fd()), unix.LOOP_SET_CAPACITY, 0))
}

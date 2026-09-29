//go:build linux

package linux

import (
	"errors"
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// A LoopDevice is an attached loop device, held open until its filesystem is
// mounted. Closing it before a mount holds the device detaches it.
type LoopDevice struct {
	number uint32
	file   *os.File
}

// Number returns the device's number, which the sandbox keeps for growth.
func (d *LoopDevice) Number() uint32 { return d.number }

// Path returns the device's path.
func (d *LoopDevice) Path() string { return LoopPath(d.number) }

// Close releases the descriptor that holds the device open.
func (d *LoopDevice) Close() error { return d.file.Close() }

// LoopPath returns /dev/loop<number>.
func LoopPath(number uint32) string {
	return fmt.Sprintf("/dev/loop%d", number)
}

// AttachLoop attaches image to a free loop device with auto-clear set, so
// unmounting its filesystem, or a mount that fails, detaches the device.
func AttachLoop(image string) (*LoopDevice, error) {
	backing, err := os.OpenFile(image, os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	defer backing.Close()
	control, err := os.OpenFile("/dev/loop-control", os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	defer control.Close()
	for {
		number, err := unix.IoctlRetInt(int(control.Fd()), unix.LOOP_CTL_GET_FREE)
		if err != nil {
			return nil, Failed("finding a free loop device through", "/dev/loop-control", err)
		}
		path := LoopPath(uint32(number))
		device, err := os.OpenFile(path, os.O_RDWR, 0)
		if err != nil {
			return nil, err
		}
		config := unix.LoopConfig{Fd: uint32(backing.Fd())}
		config.Info.Flags = unix.LO_FLAGS_AUTOCLEAR
		err = unix.IoctlLoopConfigure(int(device.Fd()), &config)
		switch {
		case err == nil:
			return &LoopDevice{number: uint32(number), file: device}, nil
		case errors.Is(err, unix.EBUSY):
			// Another process took the device between the two calls.
			device.Close()
		default:
			device.Close()
			return nil, Failed("attaching", path, err)
		}
	}
}

// RefreshLoopCapacity makes loop device number see its backing file's new size.
func RefreshLoopCapacity(number uint32) error {
	path := LoopPath(number)
	device, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	defer device.Close()
	if err := unix.IoctlSetInt(int(device.Fd()), unix.LOOP_SET_CAPACITY, 0); err != nil {
		return Failed("refreshing the capacity of", path, err)
	}
	return nil
}

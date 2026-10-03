//go:build darwin || linux

package process

import (
	"encoding/binary"
	"errors"
	"fmt"
	"os"
	"strconv"
	"syscall"

	"golang.org/x/sys/unix"
)

const bootstrapArgument = "--demi-child-bootstrap"

// runChildBootstrap applies only arguments constructed by startPlatform. No marker
// is placed in the environment, so a launched program cannot inherit the mode.
func runChildBootstrap() (bool, error) {
	args := os.Args
	if len(args) < 2 || args[1] != bootstrapArgument {
		return false, nil
	}
	if len(args) < 7 {
		return true, fmt.Errorf("invalid child bootstrap arguments")
	}
	fd, err := strconv.Atoi(args[2])
	if err != nil || fd < 3 {
		return true, fmt.Errorf("invalid child bootstrap descriptor")
	}
	// Mark before changing limits. Successful exec closes the only writer.
	unix.CloseOnExec(fd)
	err = execBootstrap(args[3:])
	var errno syscall.Errno
	if !errors.As(err, &errno) {
		errno = unix.EINVAL
	}
	var code [4]byte
	binary.LittleEndian.PutUint32(code[:], uint32(errno))
	for pending := code[:]; len(pending) > 0; {
		n, writeErr := unix.Write(fd, pending)
		if errors.Is(writeErr, unix.EINTR) {
			continue
		}
		if writeErr != nil {
			break
		} // Parent closure means cancellation; the process still exits.
		pending = pending[n:]
	}
	_ = unix.Close(fd) // Error is already delivered; no descriptor remains on success.
	return true, err
}

// execBootstrap validates launch arguments before applying any child attribute.
func execBootstrap(args []string) error {
	var mask *uint32
	if args[0] != "-" {
		value, err := strconv.ParseUint(args[0], 10, 32)
		if err != nil {
			return unix.EINVAL
		}
		m := uint32(value)
		mask = &m
	}
	count, err := strconv.Atoi(args[1])
	if err != nil || count < 0 || count > (len(args)-4)/3 {
		return unix.EINVAL
	}
	limits := make([]ResourceLimit, count)
	cursor := 2
	for i := range limits {
		resource, err := strconv.Atoi(args[cursor])
		if err != nil {
			return unix.EINVAL
		}
		soft, err := strconv.ParseUint(args[cursor+1], 10, 64)
		if err != nil {
			return unix.EINVAL
		}
		hard, err := strconv.ParseUint(args[cursor+2], 10, 64)
		if err != nil {
			return unix.EINVAL
		}
		limits[i] = ResourceLimit{Resource: resource, Soft: soft, Hard: hard}
		cursor += 3
	}
	if len(args)-cursor < 2 {
		return unix.EINVAL
	}
	if mask != nil {
		unix.Umask(int(*mask))
	}
	for _, limit := range limits {
		if err := unix.Setrlimit(limit.Resource, &unix.Rlimit{Cur: limit.Soft, Max: limit.Hard}); err != nil {
			return err
		}
	}
	return unix.Exec(args[cursor], args[cursor+1:], os.Environ())
}

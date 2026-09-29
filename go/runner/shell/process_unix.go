//go:build !windows

package shell

import (
	"errors"
	"io"
	"os"
	"os/exec"
	"syscall"

	"golang.org/x/sys/unix"
)

func brokenPipe(err error) bool { return errors.Is(err, syscall.EPIPE) }

// duplicateInput preserves nonblocking pipe flags and prevents fork from
// inheriting the descriptor between dup and close-on-exec.
func duplicateInput(file *os.File) (*os.File, error) {
	raw, err := file.SyscallConn()
	if err != nil {
		return nil, err
	}
	var fd int
	var duplicateErr error
	syscall.ForkLock.RLock()
	defer syscall.ForkLock.RUnlock()
	err = raw.Control(func(original uintptr) {
		fd, duplicateErr = syscall.Dup(int(original))
		if duplicateErr == nil {
			syscall.CloseOnExec(fd)
		}
	})
	if err != nil {
		return nil, err
	}
	if duplicateErr != nil {
		return nil, duplicateErr
	}
	return os.NewFile(uintptr(fd), file.Name()), nil
}

func processStatus(err *exec.ExitError) uint8 {
	if status, ok := err.Sys().(syscall.WaitStatus); ok && status.Signaled() {
		return uint8(128 + status.Signal())
	}
	return uint8(err.ExitCode())
}

// inputWake lets the end of a call wake a read of its stdin copy: a read polls
// the copy together with a pipe of its own, which interrupt writes to.
type inputWake struct {
	reader, writer *os.File
}

func newInputWake() (*inputWake, error) {
	reader, writer, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	return &inputWake{reader: reader, writer: writer}, nil
}

// read waits until file has input, an end of file or an error, or until
// interrupt is called, and then reads.
func (w *inputWake) read(file *os.File, p []byte) (int, error) {
	input, err := file.SyscallConn()
	if err != nil {
		return 0, err
	}
	wake, err := w.reader.SyscallConn()
	if err != nil {
		return 0, err
	}
	var n int
	var readErr, inputErr error
	wakeErr := wake.Control(func(wakeFD uintptr) {
		inputErr = input.Control(func(fd uintptr) {
			n, readErr = pollRead(int(fd), int(wakeFD), p)
		})
	})
	if err := errors.Join(wakeErr, inputErr); err != nil {
		return 0, err
	}
	return n, readErr
}

// pollRead reads fd once poll finds it ready; wakeFD becoming readable ends
// the wait instead. A descriptor that is nonblocking after all may have been
// drained by another reader meanwhile, and waits again.
func pollRead(fd, wakeFD int, p []byte) (int, error) {
	for {
		fds := []unix.PollFd{{Fd: int32(fd), Events: unix.POLLIN}, {Fd: int32(wakeFD), Events: unix.POLLIN}}
		if _, err := unix.Poll(fds, -1); err != nil {
			if err == unix.EINTR {
				continue
			}
			return 0, err
		}
		if fds[1].Revents != 0 {
			return 0, os.ErrClosed
		}
		n, err := unix.Read(fd, p)
		switch {
		case err == unix.EAGAIN || err == unix.EINTR:
			continue
		case err != nil:
			return 0, err
		case n == 0 && len(p) > 0:
			return 0, io.EOF
		}
		return n, nil
	}
}

// interrupt wakes the reads waiting on the call's stdin copy.
func (w *inputWake) interrupt(*os.File) {
	// The pipe is new and empty, so one byte never blocks; a failed write
	// leaves the reads to end with the input.
	_, _ = w.writer.Write([]byte{0})
}

// release closes the stdin copy and the wake pipe once no read uses them.
func (w *inputWake) release(file *os.File) {
	// Nothing was written to either, so their close errors cannot change the
	// command's outcome.
	_ = file.Close()
	_ = w.reader.Close()
	_ = w.writer.Close()
}

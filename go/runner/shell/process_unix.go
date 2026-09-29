//go:build !windows

package shell

import (
	"errors"
	"log/slog"
	"os"
	"os/exec"
	"syscall"
)

func contain(cmd *exec.Cmd) (func(), error) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error {
		err := syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		if errors.Is(err, syscall.ESRCH) {
			return os.ErrProcessDone
		}
		return err
	}
	return func() {
		if cmd.Process != nil {
			// The reaped leader no longer owns any descendants that remain.
			// ESRCH is normal when its process group has already emptied.
			err := syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
			if err != nil && !errors.Is(err, syscall.ESRCH) {
				slog.Warn("shell descendant cleanup failed", "error", err)
			}
		}
	}, nil
}
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

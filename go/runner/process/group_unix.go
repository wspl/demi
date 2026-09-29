//go:build unix

package process

import (
	"errors"
	"os/exec"
	"runtime"
	"syscall"
)

// Group owns containment for one command and its descendants. Do not copy it.
// Close it after a failed start or after reaping the leader. Kill may also be
// called while the process runs; the caller still owns Wait.
type Group struct{ cmd *exec.Cmd }

// Contain configures a command before its first Start. Existing process
// attributes are preserved except its process-group membership. For a command
// created by exec.CommandContext, cancellation kills the entire group.
func Contain(cmd *exec.Cmd) (*Group, error) {
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
	cmd.SysProcAttr.Pgid = 0
	group := &Group{cmd: cmd}
	if cmd.Cancel != nil {
		cmd.Cancel = group.Kill
	}
	return group, nil
}

// Kill ends the whole group. A group whose members have already exited is gone,
// including macOS EPERM while only exiting members remain (Rust process::gone).
func (g *Group) Kill() error {
	if g.cmd.Process == nil {
		return nil
	}
	err := syscall.Kill(-g.cmd.Process.Pid, syscall.SIGKILL)
	if gone(err) {
		return nil
	}
	return err
}

func gone(err error) bool {
	return errors.Is(err, syscall.ESRCH) || runtime.GOOS == "darwin" && errors.Is(err, syscall.EPERM)
}

// Close ends descendants remaining after their leader exits.
func (g *Group) Close() error { return g.Kill() }

// ExecHelper is a no-op on Unix. Programs also built for Windows call it at
// the beginning of main, before interpreting their own arguments.
func ExecHelper() {}

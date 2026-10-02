//go:build darwin || linux

package main

import (
	"errors"
	"os"
	"os/exec"
	"syscall"
)

func terminationSignal() os.Signal { return syscall.SIGTERM }

// prepareProcess isolates each suite and forwards cancellation to its children.
func prepareProcess(command *exec.Cmd) {
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	command.Cancel = func() error { return signalGroup(command, syscall.SIGTERM) }
}

// cleanupProcess stops children left behind by a failed Rust test.
func cleanupProcess(command *exec.Cmd) error { return signalGroup(command, syscall.SIGKILL) }

func signalGroup(command *exec.Cmd, signal syscall.Signal) error {
	if command.Process == nil {
		return nil
	}
	err := syscall.Kill(-command.Process.Pid, signal)
	if errors.Is(err, syscall.ESRCH) {
		return nil
	}
	return err
}

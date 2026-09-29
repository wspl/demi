//go:build unix && !linux

package procgroup

import (
	"os/exec"
	"syscall"
)

// Start starts cmd in its own process group. Without Linux's parent-death
// signal, abrupt termination of the suite cannot kill its children; normal
// cleanup must call Kill.
func Start(cmd *exec.Cmd) error {
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
	return cmd.Start()
}

//go:build unix

package procgroup

import (
	"os/exec"
	"syscall"
)

// Kill kills the process and every process left in its group. A process or a
// group that is gone already has nothing to kill.
func Kill(cmd *exec.Cmd) {
	if cmd.Process == nil {
		return
	}
	_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
	_ = cmd.Process.Kill()
}

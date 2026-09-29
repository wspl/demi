// Package procgroup starts the processes of a suite so that none outlives it:
// each leads its own process group, which a cleanup kills whole, and each dies
// with the process that started it, whether the suite ends, times out, is
// interrupted or panics.
package procgroup

import (
	"os/exec"
	"runtime"
	"sync"
	"syscall"
)

// A start is a request to the launcher goroutine.
type start struct {
	cmd  *exec.Cmd
	done chan error
}

var launcher struct {
	once     sync.Once
	requests chan start
}

// launch runs every start of the process on one goroutine locked to its OS
// thread. The kernel sends a parent-death signal when the thread that started
// a child ends, not when the process does, so the children must come from a
// thread that lives as long as the process. The goroutine never unlocks and
// never returns: it ends with the process, and a goroutine that cannot be
// stopped is documented at its start.
func launch() {
	runtime.LockOSThread()
	for request := range launcher.requests {
		request.done <- request.cmd.Start()
	}
}

// Start starts cmd in a process group of its own, with SIGKILL for it when the
// launcher thread ends, which is when this process does.
func Start(cmd *exec.Cmd) error {
	launcher.once.Do(func() {
		launcher.requests = make(chan start)
		go launch()
	})
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
	cmd.SysProcAttr.Pdeathsig = syscall.SIGKILL
	done := make(chan error, 1)
	launcher.requests <- start{cmd: cmd, done: done}
	return <-done
}

// Kill kills the process and every process left in its group. A process or a
// group that is gone already has nothing to kill.
func Kill(cmd *exec.Cmd) {
	if cmd.Process == nil {
		return
	}
	_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
	_ = cmd.Process.Kill()
}

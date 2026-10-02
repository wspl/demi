package tabs

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"syscall"
)

// start pins the spawning thread until Chrome is reaped: Linux parent-death
// signals follow the creating thread, not the Go process's other threads.
func (p *chromeProcess) start(command *exec.Cmd, profile string) error {
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true, Pdeathsig: syscall.SIGKILL}
	command.Env = append(command.Env, "DEMI_BROWSER_PROFILE="+p.runtime, "TMPDIR="+p.runtime, "CHROME_CONFIG_HOME="+profile)
	p.command = command
	p.done = make(chan struct{})
	started := make(chan error, 1)
	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		defer close(p.done)
		err := command.Start()
		started <- err
		if err == nil {
			p.waitErr = command.Wait()
		}
	}()
	err := <-started
	if err != nil {
		<-p.done
	}
	return err
}

// markedProcesses reads environments only after matching the executable root.
func markedProcesses(roots []string, marker string) ([]int, error) {
	if len(roots) == 0 {
		return nil, nil
	}
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return nil, err
	}
	var marked []int
	for _, entry := range entries {
		pid, err := strconv.Atoi(entry.Name())
		if err != nil {
			continue
		}
		base := filepath.Join("/proc", entry.Name())
		executable, err := os.Readlink(filepath.Join(base, "exe"))
		if err != nil || !installedProcess(roots, executable) {
			continue
		}
		data, err := os.ReadFile(filepath.Join(base, "environ"))
		if err != nil {
			continue
		} // Exited or inaccessible processes cannot carry a readable owned marker.
		for _, item := range bytes.Split(data, []byte{0}) {
			if string(item) == marker {
				marked = append(marked, pid)
				break
			}
		}
	}
	return marked, nil
}

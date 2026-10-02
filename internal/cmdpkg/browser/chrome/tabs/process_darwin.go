package tabs

import (
	"bytes"
	"os/exec"
	"syscall"
	"unsafe"

	"golang.org/x/sys/unix"
)

func (p *chromeProcess) start(command *exec.Cmd, _ string) error {
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	command.Env = append(command.Env, "DEMI_BROWSER_PROFILE="+p.runtime, "MAC_CHROMIUM_TMPDIR="+p.runtime)
	if err := command.Start(); err != nil {
		return err
	}
	p.command = command
	p.done = make(chan struct{})
	go func() {
		p.waitErr = command.Wait()
		close(p.done)
	}()
	return nil
}

// markedProcesses inspects an executable before requesting its environment.
func markedProcesses(roots []string, marker string) ([]int, error) {
	if len(roots) == 0 {
		return nil, nil
	}
	processes, err := unix.SysctlKinfoProcSlice("kern.proc.all")
	if err != nil {
		return nil, err
	}
	var marked []int
	for _, process := range processes {
		pid := int(process.Proc.P_pid)
		if pid <= 0 {
			continue
		}
		// x/sys exposes proc_info's syscall but not proc_pidpath. Darwin's
		// PROC_INFO_CALL_PIDINFO (2), PROC_PIDPATHINFO (11) writes at most 4096
		// bytes. The buffer is live through the synchronous syscall; no cgo is used.
		path := make([]byte, 4096)
		//nolint:staticcheck // x/sys has no proc_pidpath wrapper, and this package cannot use cgo or purego.
		_, _, errno := unix.Syscall6(unix.SYS_PROC_INFO, 2, uintptr(pid), 11, 0, uintptr(unsafe.Pointer(&path[0])), uintptr(len(path)))
		end := bytes.IndexByte(path, 0)
		if errno != 0 || end <= 0 {
			continue
		}
		executable := string(path[:end])
		if !installedProcess(roots, executable) {
			continue
		}
		args, err := unix.SysctlRaw("kern.procargs2", pid)
		if err != nil {
			continue
		} // It exited or cannot be inspected as this user.
		for _, item := range bytes.Split(args, []byte{0}) {
			if string(item) == marker {
				marked = append(marked, pid)
				break
			}
		}
	}
	return marked, nil
}

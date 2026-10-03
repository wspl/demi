package process

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"unsafe"

	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/windows"
)

type platformGroup struct {
	mu  sync.Mutex
	job windows.Handle
}

func startPlatform(
	ctx context.Context,
	template *exec.Cmd,
	group bool,
	_ ChildAttributes,
	platform *platformGroup,
) (*exec.Cmd, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	cmd := *template
	if !group {
		if err := cmd.Start(); err != nil {
			return nil, err
		}
		return &cmd, nil
	}
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			_ = windows.CloseHandle(job)
		}
	}()
	if err := setJobLimits(job); err != nil {
		return nil, err
	}
	suspendCommand(&cmd)
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	defer func() {
		if !success {
			_ = cmd.Process.Kill() // Failed assignment/resume must not leave a suspended process.
			_ = cmd.Wait()
		}
	}()
	handle, err := windows.OpenProcess(
		windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE,
		false,
		uint32(cmd.Process.Pid),
	)
	if err != nil {
		return nil, err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = windows.CloseHandle(handle) }()
	if err := windows.AssignProcessToJobObject(job, handle); err != nil {
		return nil, err
	}
	if err := resumeChild(uint32(cmd.Process.Pid)); err != nil {
		return nil, err
	}
	platform.job = job
	success = true
	return &cmd, nil
}

// resumeChild resumes the only thread of a process created suspended. os/exec
// closes the initial thread handle, so the Windows thread snapshot finds it.
func resumeChild(pid uint32) error {
	snapshot, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPTHREAD, 0)
	if err != nil {
		return err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = windows.CloseHandle(snapshot) }()
	entry := windows.ThreadEntry32{Size: uint32(unsafe.Sizeof(windows.ThreadEntry32{}))}
	for err = windows.Thread32First(snapshot, &entry); err == nil; err = windows.Thread32Next(snapshot, &entry) {
		if entry.OwnerProcessID != pid {
			continue
		}
		thread, err := windows.OpenThread(windows.THREAD_SUSPEND_RESUME, false, entry.ThreadID)
		if err != nil {
			return err
		}
		_, resumeErr := windows.ResumeThread(thread)
		return errors.Join(resumeErr, windows.CloseHandle(thread))
	}
	return fmt.Errorf("find suspended child thread: %w", err)
}

func (p *platformGroup) close() {
	p.mu.Lock()
	handle := p.job
	p.job = 0
	p.mu.Unlock()
	if handle != 0 {
		_ = windows.CloseHandle(handle)
	} // All job processes have already been terminated.
}

func (p *platformGroup) kill(process *os.Process, group bool) error {
	if group {
		// Duplicate under the lock so Close cannot invalidate a concurrent signal.
		p.mu.Lock()
		var handle windows.Handle
		var err error
		if p.job != 0 {
			err = windows.DuplicateHandle(
				windows.CurrentProcess(),
				p.job,
				windows.CurrentProcess(),
				&handle,
				0,
				false,
				windows.DUPLICATE_SAME_ACCESS,
			)
		}
		p.mu.Unlock()
		if err != nil {
			return err
		}
		if handle == 0 {
			return nil
		}
		// Cleanup follows the operation result; cancellation may already have closed it.
		defer func() { _ = windows.CloseHandle(handle) }()
		return windows.TerminateJobObject(handle, 1)
	}
	err := process.Kill()
	if errors.Is(err, os.ErrProcessDone) {
		return nil
	}
	return err
}

func (p *platformGroup) signal(process *os.Process, group bool, signal runnerwire.Signal) error {
	switch signal {
	case runnerwire.SignalTerminate, runnerwire.SignalKill, runnerwire.SignalInterrupt:
		return p.kill(process, group)
	default:
		return fmt.Errorf("unsupported Windows process signal")
	}
}
func exitSignal(_ *os.ProcessState) *string { return nil }
func runChildBootstrap() (bool, error)      { return false, nil }

// wait preserves Windows Job Object cleanup; Unix additionally reaps adopted children.
func (*platformGroup) wait(_ context.Context, _ *os.Process) error { return nil }

func setJobLimits(job windows.Handle) error {
	limits := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	limits.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	// x/sys exposes SetInformationJobObject as an untyped native buffer; the
	// documented information class fixes this struct's layout and size.
	if _, err := windows.SetInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&limits)),
		uint32(unsafe.Sizeof(limits)),
	); err != nil {
		return err
	}
	return nil
}

func suspendCommand(cmd *exec.Cmd) {
	attr := syscall.SysProcAttr{}
	if cmd.SysProcAttr != nil {
		attr = *cmd.SysProcAttr
	}
	attr.CreationFlags |= windows.CREATE_SUSPENDED
	cmd.SysProcAttr = &attr
}

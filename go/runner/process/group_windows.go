package process

import (
	"context"
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

const helperFlag = "--demi-process-contained"

type helperSetup struct {
	Job  uintptr
	Path string
	Args []string
}

// Group owns the command's Job Object. Do not copy it. Close it after a failed
// start or after reaping the leader; Kill may be called while the child runs.
type Group struct {
	cmd *exec.Cmd
	mu  sync.Mutex
	job windows.Handle
}

// Contain configures a command before its first Start. The executable must
// call ExecHelper at the beginning of main. Its private helper joins the Job
// Object before starting the target, so descendants cannot escape assignment.
func Contain(cmd *exec.Cmd) (*Group, error) {
	job, err := windows.CreateJobObject(&windows.SecurityAttributes{Length: uint32(unsafe.Sizeof(windows.SecurityAttributes{})), InheritHandle: 1}, nil)
	if err != nil {
		return nil, err
	}
	info := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	info.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	if _, err := windows.SetInformationJobObject(job, windows.JobObjectExtendedLimitInformation, uintptr(unsafe.Pointer(&info)), uint32(unsafe.Sizeof(info))); err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	executable, err := os.Executable()
	if err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	data, err := json.Marshal(helperSetup{uintptr(job), cmd.Path, cmd.Args})
	if err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	cmd.Path = executable
	cmd.Args = []string{executable, helperFlag, string(data)}
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.AdditionalInheritedHandles = append(cmd.SysProcAttr.AdditionalInheritedHandles, syscall.Handle(job))
	group := &Group{cmd: cmd, job: job}
	if cmd.Cancel != nil {
		cmd.Cancel = group.Kill
	}
	return group, nil
}

// Kill stops the helper first, including before it has joined the object, then
// terminates every member of the object. The caller still owns cmd.Wait.
func (g *Group) Kill() error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.job == 0 || g.cmd.Process == nil {
		return nil
	}
	err := g.cmd.Process.Kill()
	if errors.Is(err, os.ErrProcessDone) {
		err = nil
	}
	return errors.Join(err, windows.TerminateJobObject(g.job, 137))
}

// Close releases the Job Object, killing all descendants still in it.
func (g *Group) Close() error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.job == 0 {
		return nil
	}
	err := windows.CloseHandle(g.job)
	if err == nil {
		g.job = 0
	}
	return err
}

// ExecHelper handles the private containment invocation and returns for normal
// invocations. Call it at the beginning of main before application startup.
func ExecHelper() {
	if len(os.Args) < 2 || os.Args[1] != helperFlag {
		return
	}
	var setup helperSetup
	var err error
	if len(os.Args) != 3 {
		err = errors.New("invalid process helper arguments")
	} else {
		err = json.Unmarshal([]byte(os.Args[2]), &setup, json.RejectUnknownMembers(true))
	}
	if err == nil && (setup.Job == 0 || setup.Path == "" || len(setup.Args) == 0) {
		err = errors.New("invalid process helper arguments")
	}
	if err == nil {
		err = runHelper(setup)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	os.Exit(0)
}

func runHelper(setup helperSetup) error {
	job := windows.Handle(setup.Job)
	if err := windows.AssignProcessToJobObject(job, windows.CurrentProcess()); err != nil {
		return err
	}
	if err := windows.CloseHandle(job); err != nil {
		return err
	}
	cmd, err := Start(context.Background(), func() (*exec.Cmd, error) {
		cmd := exec.Command(setup.Path)
		cmd.Args = setup.Args
		cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
		return cmd, cmd.Start()
	})
	if err == nil {
		err = cmd.Wait()
	}
	var exited *exec.ExitError
	if errors.As(err, &exited) {
		os.Exit(exited.ExitCode())
	}
	return err
}

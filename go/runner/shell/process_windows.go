package shell

import (
	"errors"
	"os"
	"os/exec"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

// The helper joins the Job Object before starting the requested program, so
// there is no interval in which the program can spawn an uncontained child.
func contain(cmd *exec.Cmd) (func(), error) {
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
	path, args, err := helperArgs(childSetup{Path: cmd.Path, Args: cmd.Args, JobHandle: uintptr(job)})
	if err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	cmd.Path, cmd.Args = path, args
	cmd.SysProcAttr = &syscall.SysProcAttr{AdditionalInheritedHandles: []syscall.Handle{syscall.Handle(job)}}
	cmd.Cancel = func() error {
		// Stop the helper first: cancellation may precede its assignment to
		// the object. Once stopped it cannot create an uncontained successor.
		err := cmd.Process.Kill()
		if errors.Is(err, os.ErrProcessDone) {
			err = nil
		}
		return errors.Join(err, windows.TerminateJobObject(job, 137))
	}
	return func() { windows.CloseHandle(job) }, nil
}
func brokenPipe(err error) bool { return errors.Is(err, syscall.ERROR_BROKEN_PIPE) }

func duplicateInput(file *os.File) (*os.File, error) {
	process := windows.CurrentProcess()
	var handle windows.Handle
	err := windows.DuplicateHandle(process, windows.Handle(file.Fd()), process, &handle, 0, false, windows.DUPLICATE_SAME_ACCESS)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(handle), file.Name()), nil
}

func processStatus(err *exec.ExitError) uint8 { return uint8(err.ExitCode()) }

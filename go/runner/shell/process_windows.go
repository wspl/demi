package shell

import (
	"errors"
	"os"
	"os/exec"
	"syscall"

	"golang.org/x/sys/windows"
)

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

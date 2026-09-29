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

// inputWake ends a read of a call's stdin copy by closing the copy.
type inputWake struct{}

func newInputWake() (*inputWake, error) { return &inputWake{}, nil }

func (*inputWake) read(file *os.File, p []byte) (int, error) { return file.Read(p) }

func (*inputWake) interrupt(file *os.File) {
	// Closing an already failed pipe releases it too; there is no buffered
	// write to flush or close error that can change the command's outcome.
	_ = file.Close()
}

func (*inputWake) release(*os.File) {}

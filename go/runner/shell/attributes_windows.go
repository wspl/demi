package shell

import (
	"context"
	"errors"
	"os"
	"os/exec"

	"github.com/wspl/demi/go/runner/process"
	"golang.org/x/sys/windows"
)

var resourceNumbers = map[rune]int{}

const pipeBuffer = 512

func infinity() uint64 { return ^uint64(0) }
func resourceLimit(context.Context, int) (uint64, uint64, error) {
	return 0, 0, errors.New("resource limits are not supported on Windows")
}
func executeHelper(setup childSetup) error {
	if setup.JobHandle == 0 || setup.Mask != nil || len(setup.Limits) != 0 || setup.Mode != "exec" {
		return errors.New("child attributes are not supported on Windows")
	}
	job := windows.Handle(setup.JobHandle)
	if err := windows.AssignProcessToJobObject(job, windows.CurrentProcess()); err != nil {
		return err
	}
	windows.CloseHandle(job)
	cmd, err := process.Start(context.Background(), func() (*exec.Cmd, error) {
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

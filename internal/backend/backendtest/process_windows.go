package backendtest

import (
	"errors"

	"golang.org/x/sys/windows"
)

// stillActive is the exit code Windows reports for a running process.
const stillActive = 259

// ProcessRunning reports whether the process pid still exists, as a scenario
// checks that a job's process ended with its job.
func ProcessRunning(pid int) (bool, error) {
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	// The handle was only needed for the query; a close failure changes no answer.
	defer func() {
		_ = windows.CloseHandle(handle)
	}()
	var code uint32
	if err := windows.GetExitCodeProcess(handle, &code); err != nil {
		return false, err
	}
	return code == stillActive, nil
}

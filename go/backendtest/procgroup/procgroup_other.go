//go:build !unix

package procgroup

import (
	"fmt"
	"os/exec"
	"runtime"
)

// Start refuses platforms without the Unix process groups this harness needs.
// No process is started, so a failed Start leaves nothing to clean up.
func Start(cmd *exec.Cmd) error {
	return fmt.Errorf("backendtest/procgroup: process-group cleanup is unsupported on %s; run the suite on Linux", runtime.GOOS)
}

// Kill has nothing to clean up because Start refuses to start a process.
func Kill(cmd *exec.Cmd) {}

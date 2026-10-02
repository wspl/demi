package shell

import (
	"io"
	"os/exec"
)

// extraDescriptors leaves Windows descriptor inheritance to standard IO.
func extraDescriptors(_ *exec.Cmd, _ map[string]io.ReadWriteCloser) error { return nil }

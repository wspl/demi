//go:build !windows

package hostremotetest

import (
	"os"
	"syscall"
)

func terminate(process *os.Process) error { return process.Signal(syscall.SIGTERM) }

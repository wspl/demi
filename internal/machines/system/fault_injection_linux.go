//go:build linux && fault_injection

package system

import (
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// FaultPoint aborts the process here when DEMI_MACHINE_MANAGER_FAULT names this point.
func FaultPoint(name string) {
	if fault, set := os.LookupEnv("DEMI_MACHINE_MANAGER_FAULT"); set && fault == name {
		// There is no recoverable logging failure on this deliberate crash path.
		_, _ = fmt.Fprintf(os.Stderr, "demi-machine-manager: injected fault at %s\n", name)
		_ = unix.Kill(os.Getpid(), unix.SIGABRT)
		// Do not let the caller publish more state before signal delivery. The Go
		// runtime handles SIGABRT as a fatal signal; no deferred cleanup runs.
		select {}
	}
}

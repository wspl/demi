//go:build faultinjection

// Package fault holds the crash points of acceptance
// (docs/cloud/managed-hosts.md § Verification): a manager built with the
// faultinjection tag aborts at the point DEMI_MACHINES_FAULT names, and the next
// start must recover with nothing left behind. Other builds carry none of it.
package fault

import (
	"fmt"
	"os"
	"syscall"
)

// Point aborts the process here when this point is the injected fault.
func Point(name string) {
	if fault, ok := os.LookupEnv("DEMI_MACHINES_FAULT"); ok && fault == name {
		fmt.Fprintf(os.Stderr, "demi-machines: injected fault at %s\n", name)
		// A crash, not an exit: no deferred cleanup runs, and Go would turn SIGABRT
		// into an orderly exit.
		syscall.Kill(os.Getpid(), syscall.SIGKILL)
		select {}
	}
}

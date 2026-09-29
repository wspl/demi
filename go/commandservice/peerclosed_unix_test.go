//go:build !windows

package commandservice_test

import "syscall"

// closedPipeErrors are what a write into a pipe or socket whose reader is gone
// returns on this platform.
var closedPipeErrors = []error{syscall.EPIPE, syscall.ECONNRESET}

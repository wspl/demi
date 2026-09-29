//go:build windows

package commandservice_test

import "syscall"

// closedPipeErrors are what a write into a pipe whose reader is gone returns on
// this platform: ERROR_BROKEN_PIPE, ERROR_NO_DATA and ERROR_PIPE_NOT_CONNECTED,
// by their numbers in winerror.h.
var closedPipeErrors = []error{syscall.Errno(109), syscall.Errno(232), syscall.Errno(233)}

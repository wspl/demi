//go:build unix

package commandservice

import "syscall"

// errNoDescriptor is the error of a process that has no descriptor left.
var errNoDescriptor error = syscall.EMFILE

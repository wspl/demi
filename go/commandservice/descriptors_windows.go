package commandservice

import (
	"syscall"

	"golang.org/x/sys/windows"
)

// errNoDescriptor is the error of a process that has no descriptor left.
var errNoDescriptor error = syscall.Errno(windows.ERROR_TOO_MANY_OPEN_FILES)

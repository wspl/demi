package process_test

import "syscall"

func exhaustedError() error { return syscall.ERROR_TOO_MANY_OPEN_FILES }

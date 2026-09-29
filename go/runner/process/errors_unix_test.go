//go:build unix

package process_test

import "syscall"

func exhaustedError() error { return syscall.EMFILE }

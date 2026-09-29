package process_test

import "golang.org/x/sys/windows"

func exhaustedError() error { return windows.ERROR_TOO_MANY_OPEN_FILES }

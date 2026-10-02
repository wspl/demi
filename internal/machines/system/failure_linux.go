//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

// Failed describes a failed system call on path, preserving its cause for errors.Is/As.
func Failed(action, path string, err error) error { panic("not written: m-system") }

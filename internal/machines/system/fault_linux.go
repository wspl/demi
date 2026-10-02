//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

// FaultPoint aborts the process here when DEMI_MACHINE_MANAGER_FAULT names
// this point in a build with the fault_injection tag. Other builds ignore it.
func FaultPoint(name string) { panic("not written: m-system") }

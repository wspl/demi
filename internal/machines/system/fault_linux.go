//go:build linux && !fault_injection

package system

// FaultPoint aborts the process here when DEMI_MACHINE_MANAGER_FAULT names
// this point in a build with the fault_injection tag. Other builds ignore it.
func FaultPoint(_ string) {}

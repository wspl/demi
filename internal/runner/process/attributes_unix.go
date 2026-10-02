//go:build darwin || linux

package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

// RaiseOpenFileLimit raises the runner's soft descriptor limit as far as the
// system allows and remembers the original child limits. It returns the old
// and new soft limits. Call it during runner startup.
func RaiseOpenFileLimit() (before, after uint64, err error) { panic("not written: r-process") }

// ChildLimit returns a resource's inherited soft and hard limits, using the
// runner's startup open-file limits for RLIMIT_NOFILE.
func ChildLimit(resource int) (soft, hard uint64, err error) { panic("not written: r-process") }

// Umask reads and caches the runner's creation mask. Call it during startup
// before any jobs run; nothing in the runner changes the mask afterwards.
func Umask() uint32 { panic("not written: r-process") }

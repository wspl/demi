//go:build darwin || linux

package process

import "sync"

var inheritedUmask = sync.OnceValue(readUmask)

// The Go runtime has already rewritten RLIMIT_NOFILE before package startup.
// Its cached launch-time value is not exposed by a public API; these two
// entry points await the startup-limit design ruling recorded in the report.

// RaiseOpenFileLimit raises the runner's soft descriptor limit as far as the
// system allows and remembers the original child limits. It returns the old
// and new soft limits. Call it during runner startup.
func RaiseOpenFileLimit() (before, after uint64, err error) { panic("not written: r-process") }

// ChildLimit returns a resource's inherited soft and hard limits, using the
// runner's startup open-file limits for RLIMIT_NOFILE.
func ChildLimit(_ int) (soft, hard uint64, err error) { panic("not written: r-process") }

// Umask reads and caches the runner's creation mask. Call it during startup
// before any jobs run; nothing in the runner changes the mask afterwards.
func Umask() uint32 { return inheritedUmask() }

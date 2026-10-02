//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// PrepareCgroups requires CPU, memory and PID controllers before changing the
// cgroup root, then enables them for sandbox cgroups. Do not call with limits off.
func PrepareCgroups(ctx context.Context) error { panic("not written: m-sandbox") }

// Fence kills all writers in id's cgroup, waits for it to empty and removes
// it, including a partial runtime runsc never recorded. Absence is accepted.
// Do not call with limits off.
func Fence(ctx context.Context, id ID) error { panic("not written: m-sandbox") }

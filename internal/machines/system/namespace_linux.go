//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// Namespace selects the namespace a job runs in. Use HostMount, Network or NewNetwork.
type Namespace struct{}

// HostMount selects PID 1's mount namespace, where namespace handles live.
func HostMount() Namespace { panic("not written: m-system") }

// Network selects the network namespace bound at path, such as /run/netns/demi-3.
func Network(path string) Namespace { panic("not written: m-system") }

// NewNetwork selects a new network namespace, which the job may bind to a path.
func NewNetwork() Namespace { panic("not written: m-system") }

// RunNamespace runs the whole job in an owned goroutine that locks its OS
// thread, unshares CLONE_FS, enters namespace and exits without unlocking.
// It waits for the job even on cancellation. The job must close namespace-bound
// resources before returning and must do namespace work on that goroutine.
// A recovery child must be started and waited for on that same goroutine.
// Panics from the job are recovered and returned as errors.
func RunNamespace[T any](ctx context.Context, namespace Namespace, job func(context.Context) (T, error)) (T, error) {
	panic("not written: m-system")
}

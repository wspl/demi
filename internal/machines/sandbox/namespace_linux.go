//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"os"
)

// RecoveryLockFD is the descriptor on which the recovery child receives the
// manager's state lock. The manager validates this inherited lock before recovery.
const RecoveryLockFD = 3

// SavedNamespace owns the pinned mount namespace and its state-directory
// ownership record. The manager already runs in a private mount namespace.
type SavedNamespace struct{}

// NewSavedNamespace selects the manager's runtime and canonical state directory.
func NewSavedNamespace(runtime, data string) *SavedNamespace { panic("not written: m-sandbox") }

// Recover enters an earlier manager's saved namespace through system.RunNamespace
// and starts /proc/self/exe from that locked goroutine, then waits there for the
// child. It inherits the environment and args after argv[0], removes --recover,
// and adds --recover-namespace. stateLock is borrowed and inherited at descriptor
// RecoveryLockFD; the caller keeps it open until Recover returns. Recovery has
// no deadline. Success releases the handle; failure retains it for the next start.
func (n *SavedNamespace) Recover(ctx context.Context, stateLock *os.File, args []string) error {
	panic("not written: m-sandbox")
}

// Pin records the state directory and binds this manager's mount namespace
// from PID 1's namespace, making the runtime mount private first.
func (n *SavedNamespace) Pin(ctx context.Context) error { panic("not written: m-sandbox") }

// Release unmounts the handle in the host and current namespaces, and removes
// the handle and owner record. Missing resources are accepted.
func (n *SavedNamespace) Release(ctx context.Context) error { panic("not written: m-sandbox") }

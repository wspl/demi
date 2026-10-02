//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"net/netip"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// NewID creates a boot identity with the demi- prefix and a random UUID.
func NewID() (ID, error) { panic("not written: m-sandbox") }

// String returns the boot identity.
func (id ID) String() string { panic("not written: m-sandbox") }

// RuntimeDirectory is a boot's private directory under the manager's runtime root.
type RuntimeDirectory struct{}

// NewRuntimeDirectory names the runtime directory for id.
func NewRuntimeDirectory(runtime string, id ID) RuntimeDirectory { panic("not written: m-sandbox") }

// MountPoints returns mount names in release order: overlay before its backing filesystems.
func MountPoints() [5]string { panic("not written: m-sandbox") }

// Root returns the OCI bundle directory.
func (d RuntimeDirectory) Root() string { panic("not written: m-sandbox") }

// Base returns the read-only bind of the base's root.
func (d RuntimeDirectory) Base() string { panic("not written: m-sandbox") }

// Volume returns where a working image is mounted.
func (d RuntimeDirectory) Volume(volume machinewire.Volume) string { panic("not written: m-sandbox") }

// Home returns the mounted home filesystem.
func (d RuntimeDirectory) Home() string { panic("not written: m-sandbox") }

// RootFS returns the overlay the sandbox sees as /.
func (d RuntimeDirectory) RootFS() string { panic("not written: m-sandbox") }

// Credentials returns the private tmpfs of the boot's credential files.
func (d RuntimeDirectory) Credentials() string { panic("not written: m-sandbox") }

// Boot returns the managed boot credential's host path.
func (d RuntimeDirectory) Boot() string { panic("not written: m-sandbox") }

// Resolver returns the generated resolv.conf path.
func (d RuntimeDirectory) Resolver() string { panic("not written: m-sandbox") }

// Hosts returns the generated hosts file path.
func (d RuntimeDirectory) Hosts() string { panic("not written: m-sandbox") }

// Config returns the OCI bundle's configuration path.
func (d RuntimeDirectory) Config() string { panic("not written: m-sandbox") }

// Log returns runsc's own output path when it starts the sandbox.
func (d RuntimeDirectory) Log() string { panic("not written: m-sandbox") }

// Create creates the directory, private to root, and its mount points.
func (d RuntimeDirectory) Create(ctx context.Context) error { panic("not written: m-sandbox") }

// Remove removes entries without descending into mount points. A surviving
// mount keeps its contents; an already absent directory is accepted.
func (d RuntimeDirectory) Remove(ctx context.Context) error { panic("not written: m-sandbox") }

// WriteCredentials writes the boot, resolver and hosts files on mounted tmpfs
// with exact modes independent of umask. The boot file belongs to system.UserID.
func (d RuntimeDirectory) WriteCredentials(ctx context.Context, boot runnerwire.ManagedBoot, dns []netip.Addr) error {
	panic("not written: m-sandbox")
}

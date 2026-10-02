//go:build linux

package sandbox

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// NewID creates a boot identity with the demi- prefix and a random UUID.
func NewID() (ID, error) {
	var id [16]byte
	if _, err := rand.Read(id[:]); err != nil {
		return "", err
	}
	id[6] = id[6]&0x0f | 0x40
	id[8] = id[8]&0x3f | 0x80
	return ParseID(fmt.Sprintf("demi-%x-%x-%x-%x-%x", id[:4], id[4:6], id[6:8], id[8:10], id[10:]))
}

// String returns the boot identity.
func (id ID) String() string {
	return string(id)
}

// RuntimeDirectory is a boot's private directory under the manager's runtime root.
type RuntimeDirectory struct{ root string }

// NewRuntimeDirectory names the runtime directory for id.
func NewRuntimeDirectory(runtime string, id ID) RuntimeDirectory {
	return RuntimeDirectory{root: filepath.Join(runtime, string(id))}
}

// MountPoints returns mount names in release order: overlay before its backing filesystems.
func MountPoints() [5]string {
	return [5]string{"rootfs", "home", "system", "base", "credentials"}
}

// Root returns the OCI bundle directory.
func (d RuntimeDirectory) Root() string {
	return d.root
}

// Base returns the read-only bind of the base's root.
func (d RuntimeDirectory) Base() string {
	return filepath.Join(d.root, "base")
}

// Volume returns where a working image is mounted.
func (d RuntimeDirectory) Volume(volume machinewire.Volume) string {
	return filepath.Join(d.root, string(volume))
}

// Home returns the mounted home filesystem.
func (d RuntimeDirectory) Home() string {
	return d.Volume(machinewire.VolumeHome)
}

// RootFS returns the overlay the sandbox sees as /.
func (d RuntimeDirectory) RootFS() string {
	return filepath.Join(d.root, "rootfs")
}

// Credentials returns the private tmpfs of the boot's credential files.
func (d RuntimeDirectory) Credentials() string {
	return filepath.Join(d.root, "credentials")
}

// Boot returns the managed boot credential's host path.
func (d RuntimeDirectory) Boot() string {
	return filepath.Join(d.Credentials(), "boot.json")
}

// Resolver returns the generated resolv.conf path.
func (d RuntimeDirectory) Resolver() string {
	return filepath.Join(d.Credentials(), "resolv.conf")
}

// Hosts returns the generated hosts file path.
func (d RuntimeDirectory) Hosts() string {
	return filepath.Join(d.Credentials(), "hosts")
}

// Config returns the OCI bundle's configuration path.
func (d RuntimeDirectory) Config() string {
	return filepath.Join(d.root, "config.json")
}

// Log returns runsc's own output path when it starts the sandbox.
func (d RuntimeDirectory) Log() string {
	return filepath.Join(d.root, "runtime.log")
}

// Create creates the directory, private to root, and its mount points.
func (d RuntimeDirectory) Create(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := os.Mkdir(d.root, 0777); err != nil {
		return err
	}
	if err := os.Chmod(d.root, 0700); err != nil {
		return err
	}
	for _, name := range MountPoints() {
		if err := os.Mkdir(filepath.Join(d.root, name), 0777); err != nil {
			return err
		}
	}
	return nil
}

// Remove removes entries without descending into mount points. A surviving
// mount keeps its contents; an already absent directory is accepted.
func (d RuntimeDirectory) Remove(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	entries, err := os.ReadDir(d.root)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, entry := range entries {
		// os.Remove uses rmdir for directories: never traverse a surviving mount.
		if err := os.Remove(filepath.Join(d.root, entry.Name())); err != nil {
			return err
		}
	}
	return os.Remove(d.root)
}

// WriteCredentials writes the boot, resolver and hosts files on mounted tmpfs
// with exact modes independent of umask. The boot file belongs to system.UserID.
func (d RuntimeDirectory) WriteCredentials(ctx context.Context, boot runnerwire.ManagedBoot, dns []netip.Addr) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	var resolver strings.Builder
	for _, address := range dns {
		// strings.Builder writes cannot fail.
		fmt.Fprintf(&resolver, "nameserver %s\n", address)
	}
	if err := createCredential(ctx, d.Resolver(), []byte(resolver.String()), 0444); err != nil {
		return err
	}
	if err := createCredential(ctx, d.Hosts(), []byte("127.0.0.1 localhost\n127.0.1.1 demi-cloud\n"), 0444); err != nil {
		return err
	}
	data, err := contract.EncodeJSON(boot)
	if err != nil {
		return err
	}
	if err := createCredential(ctx, d.Boot(), data, 0400); err != nil {
		return err
	}
	return os.Chown(d.Boot(), int(system.UserID), int(system.UserID))
}

// createCredential creates a runtime credential file with its exact read mode.
func createCredential(ctx context.Context, path string, data []byte, mode os.FileMode) (err error) {
	if err := ctx.Err(); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, file.Close()) }()
	if _, err := file.Write(data); err != nil {
		return err
	}
	return file.Chmod(mode)
}

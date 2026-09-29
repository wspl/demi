// Package sandbox is one boot of a device's sandbox (docs/cloud/managed-hosts.md
// § Container initialization): its resources are acquired in a fixed order, each
// recorded where recovery finds it, and released in reverse by [Sandbox.Close],
// which tolerates every absence, so it also cleans up a failed start and a boot
// recovery found.
package sandbox

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"errors"
	"io/fs"
	"net/netip"
	"os"
	"path/filepath"
	"strings"
	"uuid"

	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/runnerproto"
)

// An InvalidError means a record breaks the rules of its wire type. It names the
// field and the rule, never the value.
type InvalidError = wire.InvalidError

// A SandboxID is one boot's id, "demi-" and a UUID: it names the runtime
// directory, the runsc container and the cgroup.
type SandboxID string

// NewSandboxID returns the id of a new boot.
func NewSandboxID() SandboxID {
	return SandboxID("demi-" + uuid.New().String())
}

// sandboxID is the rule of a boot's id.
func sandboxID(id SandboxID) error {
	name, ok := strings.CutPrefix(string(id), "demi-")
	if !ok || !machinesproto.IsImageName(name) {
		return errors.New("is not a sandbox id")
	}
	return nil
}

// A Record is sandbox.json in the working pair: written before any resource of
// the boot exists and removed after the last is released, so its presence means
// recovery must fence the boot. It holds no credential.
//
//demi:wire
type Record struct {
	ID   SandboxID `json:"id" check:"func=sandboxID"`
	Slot uint16    `json:"slot"`
}

// DecodeRecord decodes a runtime record.
func DecodeRecord(data []byte) (Record, error) {
	return decode[Record](data)
}

// EncodeRecord returns a runtime record's JSON.
func EncodeRecord(record Record) ([]byte, error) {
	return encode(record)
}

// UserID is the UID and GID of the sandbox's user, demi.
const UserID = storage.UserID

// A RuntimeDirectory is a boot's runtime directory, /run/demi-machines/<sandbox>.
//
//	base/ system/ home/ rootfs/ credentials/   mount points
//	config.json runtime.log                    the OCI bundle and runsc's output
type RuntimeDirectory struct {
	root string
}

// MountPoints are the mount points in the order they are released: the overlay
// first, the filesystems it stacks on after it.
var MountPoints = [...]string{"rootfs", "home", "system", "base", "credentials"}

// NewRuntimeDirectory returns the directory of sandbox under runtime.
func NewRuntimeDirectory(runtime, sandbox string) *RuntimeDirectory {
	return &RuntimeDirectory{root: filepath.Join(runtime, sandbox)}
}

// Root returns the directory.
func (d *RuntimeDirectory) Root() string { return d.root }

// Base returns the read-only bind of the base's root.
func (d *RuntimeDirectory) Base() string { return filepath.Join(d.root, "base") }

// Volume returns where a working image is mounted.
func (d *RuntimeDirectory) Volume(volume machinesproto.Volume) string {
	return filepath.Join(d.root, string(volume))
}

// Home returns where the home image is mounted.
func (d *RuntimeDirectory) Home() string { return d.Volume(machinesproto.Home) }

// RootFS returns the overlay the sandbox sees as /.
func (d *RuntimeDirectory) RootFS() string { return filepath.Join(d.root, "rootfs") }

// Credentials returns the private tmpfs of the boot's credential files.
func (d *RuntimeDirectory) Credentials() string { return filepath.Join(d.root, "credentials") }

// Boot returns the boot record's file.
func (d *RuntimeDirectory) Boot() string { return filepath.Join(d.Credentials(), "boot.json") }

// Resolver returns the resolver configuration's file.
func (d *RuntimeDirectory) Resolver() string { return filepath.Join(d.Credentials(), "resolv.conf") }

// Hosts returns the hosts file.
func (d *RuntimeDirectory) Hosts() string { return filepath.Join(d.Credentials(), "hosts") }

// Config returns the OCI bundle's configuration.
func (d *RuntimeDirectory) Config() string { return filepath.Join(d.root, "config.json") }

// Log returns runsc's own output when it starts the sandbox.
func (d *RuntimeDirectory) Log() string { return filepath.Join(d.root, "runtime.log") }

// Create creates the directory, private to root, and its mount points.
func (d *RuntimeDirectory) Create() error {
	if err := os.Mkdir(d.root, 0o777); err != nil {
		return err
	}
	if err := os.Chmod(d.root, 0o700); err != nil {
		return err
	}
	for _, name := range MountPoints {
		if err := os.Mkdir(filepath.Join(d.root, name), 0o777); err != nil {
			return err
		}
	}
	return nil
}

// Remove removes the directory entry by entry, so a mount that survived keeps its
// contents: removing its mount point fails instead. A directory that is gone
// already is fine.
func (d *RuntimeDirectory) Remove() error {
	entries, err := os.ReadDir(d.root)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, entry := range entries {
		// Remove tries the entry as a file, then as an empty directory, never
		// recursively.
		if err := os.Remove(filepath.Join(d.root, entry.Name())); err != nil {
			return err
		}
	}
	return os.Remove(d.root)
}

// WriteCredentials writes the credential files on the mounted credentials tmpfs,
// each with its mode set whatever the service's umask: the boot record readable
// by the sandbox's user alone, the resolver and hosts files by everyone.
func (d *RuntimeDirectory) WriteCredentials(boot runnerproto.ManagedBoot, dns []netip.Addr) error {
	var resolver strings.Builder
	for _, address := range dns {
		resolver.WriteString("nameserver " + address.String() + "\n")
	}
	record, err := boot.Encode()
	if err != nil {
		return err
	}
	if err := createFile(d.Resolver(), []byte(resolver.String()), 0o444); err != nil {
		return err
	}
	if err := createFile(d.Hosts(), []byte("127.0.0.1 localhost\n127.0.1.1 demi-cloud\n"), 0o444); err != nil {
		return err
	}
	if err := createFile(d.Boot(), record, 0o400); err != nil {
		return err
	}
	return os.Chown(d.Boot(), UserID, UserID)
}

// createFile makes a new file with data and exactly mode.
func createFile(path string, data []byte, mode fs.FileMode) error {
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	if _, err := file.Write(data); err != nil {
		file.Close()
		return err
	}
	if err := file.Chmod(mode); err != nil {
		file.Close()
		return err
	}
	return file.Close()
}

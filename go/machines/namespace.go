//go:build linux

package machines

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"

	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/storage"
)

// An OwnerError means the saved namespace belongs to a manager with another state
// directory.
type OwnerError struct {
	DataDir string
}

func (e *OwnerError) Error() string {
	return "Recover the previous Cloud manager with its original state directory: " + e.DataDir
}

// A NamespaceRecoveryError means the recovery process failed.
type NamespaceRecoveryError struct {
	Status string
}

func (e *NamespaceRecoveryError) Error() string {
	return "Cloud namespace recovery failed: " + e.Status
}

// A SavedNamespace is the saved mount namespace (docs/cloud/managed-hosts.md §
// Startup and recovery): the manager pins its private mount namespace with a
// handle in the execution host's namespace, so the filesystems it froze or
// mounted stay reachable if it dies, and the next manager recovers them through
// it.
//
//	/run/demi-machines/mount-namespace              the handle: a bind of /proc/<pid>/ns/mnt
//	/run/demi-machines/mount-namespace-owner.json   {"dataDir": ...}: whose state it holds
type SavedNamespace struct {
	runtime string
	handle  string
	owner   string
	data    string
}

// NewSavedNamespace returns the saved namespace of a manager with the state
// directory data and the runtime directory runtime.
func NewSavedNamespace(runtime, data string) *SavedNamespace {
	return &SavedNamespace{
		runtime: runtime,
		handle:  filepath.Join(runtime, "mount-namespace"),
		owner:   filepath.Join(runtime, "mount-namespace-owner.json"),
		data:    data,
	}
}

// hostMount is the execution host's mount namespace, PID 1's: the namespace
// handle lives there.
const hostMount = "/proc/1/ns/mnt"

// Recover recovers a namespace an earlier manager left: a process of this
// executable, started inside it with the state lock, thaws and fences what it
// holds and saves every working pair; then the handle goes. A failed recovery
// keeps the handle for the next start.
func (n *SavedNamespace) Recover(ctx context.Context, lock *ManagerLock) error {
	var saved *os.File
	err := linux.InMountNamespace(hostMount, func() error {
		root, err := linux.IsMountRoot(n.handle)
		if err != nil || !root {
			return err
		}
		saved, err = os.Open(n.handle)
		return err
	})
	if err != nil || saved == nil {
		return err
	}
	// Opened through the handle, the descriptor keeps the handle's mount busy;
	// closed, it lets the release unmount the handle.
	defer saved.Close()
	data, err := os.ReadFile(n.owner)
	if err != nil {
		return err
	}
	owner, err := decode[namespaceOwner](data)
	if err != nil {
		return fmt.Errorf("%s is not a valid namespace owner record: %w", n.owner, err)
	}
	if owner.DataDir != n.data {
		return &OwnerError{DataDir: owner.DataDir}
	}
	if err := runRecovery(ctx, saved, lock); err != nil {
		return err
	}
	if err := saved.Close(); err != nil {
		return err
	}
	return n.Release()
}

// runRecovery runs this executable with --recover-namespace inside saved and
// waits for it. The state lock is descriptor 3 when it starts. The arguments are
// this process's, less --recover; the environment, where the service's settings
// are, is inherited.
func runRecovery(ctx context.Context, saved *os.File, lock *ManagerLock) error {
	args := slices.DeleteFunc(slices.Clone(os.Args[1:]), func(arg string) bool { return arg == "--recover" })
	args = append(args, "--recover-namespace")
	// Recovery is the manager's own code, not a tool.
	command := exec.CommandContext(ctx, "/proc/self/exe", args...)
	command.Stdout = os.Stdout
	command.Stderr = os.Stderr
	command.ExtraFiles = []*os.File{lock.data}
	// Go forks from the calling thread, so the child starts in the namespace the
	// thread entered.
	if err := linux.InMountNamespaceFile(saved, command.Start); err != nil {
		return err
	}
	if err := command.Wait(); err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) {
			return &NamespaceRecoveryError{Status: exit.ProcessState.String()}
		}
		return err
	}
	return nil
}

// Pin pins this manager's namespace: its runtime directory becomes a private
// mount in the host's namespace (nsfs refuses a handle under a shared mount), the
// owner record names the state directory, and the handle binds this process's
// namespace from the host's side (a namespace cannot hold a handle to itself).
func (n *SavedNamespace) Pin() error {
	err := linux.InMountNamespace(hostMount, func() error {
		root, err := linux.IsMountRoot(n.runtime)
		if err != nil {
			return err
		}
		if !root {
			if err := linux.Bind(n.runtime, n.runtime); err != nil {
				return err
			}
		}
		return linux.MakePrivate(n.runtime)
	})
	if err != nil {
		return err
	}
	handle, err := os.OpenFile(n.handle, os.O_WRONLY|os.O_CREATE, 0o600)
	if err != nil {
		return err
	}
	if err := handle.Close(); err != nil {
		return err
	}
	record, err := encode(namespaceOwner{DataDir: n.data})
	if err != nil {
		return err
	}
	if err := storage.WriteRecord(n.owner, record); err != nil {
		return err
	}
	if err := storage.Sync(filepath.Dir(n.owner)); err != nil {
		return err
	}
	own := "/proc/" + strconv.Itoa(os.Getpid()) + "/ns/mnt"
	return linux.InMountNamespace(hostMount, func() error {
		return linux.Bind(own, n.handle)
	})
}

// Release releases the handle, in the host's namespace and in this one, where a
// copy may have been inherited, and removes it with its owner record.
func (n *SavedNamespace) Release() error {
	err := linux.InMountNamespace(hostMount, func() error {
		root, err := linux.IsMountRoot(n.handle)
		if err != nil || !root {
			return err
		}
		return linux.Unmount(n.handle)
	})
	if err != nil {
		return err
	}
	root, err := linux.IsMountRoot(n.handle)
	if err != nil {
		return err
	}
	if root {
		if err := linux.Unmount(n.handle); err != nil {
			return err
		}
	}
	for _, file := range []string{n.handle, n.owner} {
		if err := os.Remove(file); err != nil && !errors.Is(err, fs.ErrNotExist) {
			return err
		}
	}
	return nil
}

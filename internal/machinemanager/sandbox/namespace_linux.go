//go:build linux

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/machinemanager/system"
)

// RecoveryLockFD is the descriptor on which the recovery child receives the
// manager's state lock. The manager validates this inherited lock before recovery.
const RecoveryLockFD = 3

// SavedNamespace owns the pinned mount namespace and its state-directory
// ownership record. The manager already runs in a private mount namespace.
type SavedNamespace struct {
	runtime string
	data    string
}

// NewSavedNamespace selects the manager's runtime and canonical state directory.
func NewSavedNamespace(runtime, data string) *SavedNamespace {
	return &SavedNamespace{runtime: runtime, data: data}
}

// Recover enters an earlier manager's saved namespace through system.RunNamespace
// and starts /proc/self/exe from that locked goroutine, then waits there for the
// child. It inherits the environment and args after argv[0], removes --recover,
// and adds --recover-namespace. stateLock is borrowed and inherited at descriptor
// RecoveryLockFD; the caller keeps it open until Recover returns. Recovery has
// no deadline. Success releases the handle; failure retains it for the next start.
func (n *SavedNamespace) Recover(ctx context.Context, stateLock *os.File, args []string) (err error) {
	handle := filepath.Join(n.runtime, "mount-namespace")
	saved, err := system.RunNamespace(ctx, system.HostMount(), func(ctx context.Context) (*os.File, error) {
		mounted, _, err := system.MountRoot(ctx, handle)
		if err != nil || !mounted {
			return nil, err
		}
		return os.Open(handle)
	})
	if err != nil || saved == nil {
		return err
	}
	defer func() {
		if saved != nil {
			err = errors.Join(err, saved.Close())
		}
	}()
	ownerPath := filepath.Join(n.runtime, "mount-namespace-owner.json")
	data, err := os.ReadFile(ownerPath)
	if err != nil {
		return err
	}
	owner, err := decodeNamespaceOwner(data)
	if err != nil {
		return fmt.Errorf("%s is not a valid namespace owner record: %w", ownerPath, err)
	}
	if owner.DataDir != n.data {
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return fmt.Errorf("Recover the previous Cloud manager with its original state directory: %s", owner.DataDir)
	}
	_, err = system.RunNamespace(ctx, system.Mount(saved), func(ctx context.Context) (struct{}, error) {
		return struct{}{}, runRecoveryChild(ctx, stateLock, args)
	})
	if err != nil {
		return err
	}
	// This descriptor holds the bind mount busy until it closes.
	err = saved.Close()
	saved = nil
	if err != nil {
		return err
	}
	return n.Release(ctx)
}

// Pin records the state directory and binds this manager's mount namespace
// from PID 1's namespace, making the runtime mount private first.
func (n *SavedNamespace) Pin(ctx context.Context) (err error) {
	own, err := os.Open("/proc/thread-self/ns/mnt")
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, own.Close())
	}()
	_, err = system.RunNamespace(ctx, system.HostMount(), func(ctx context.Context) (struct{}, error) {
		mounted, _, err := system.MountRoot(ctx, n.runtime)
		if err != nil {
			return struct{}{}, err
		}
		if !mounted {
			if err := system.Bind(ctx, n.runtime, n.runtime); err != nil {
				return struct{}{}, err
			}
		}
		return struct{}{}, system.MakePrivate(ctx, n.runtime)
	})
	if err != nil {
		return err
	}
	handle := filepath.Join(n.runtime, "mount-namespace")
	file, err := os.OpenFile(handle, os.O_WRONLY|os.O_CREATE, 0o600)
	if err != nil {
		return err
	}
	if err := file.Close(); err != nil {
		return err
	}
	owner, err := (namespaceOwner{DataDir: n.data}).MarshalJSON()
	if err != nil {
		return err
	}
	if err := artifacts.PublishBytes(
		ctx,
		filepath.Join(n.runtime, "mount-namespace-owner.json"),
		owner,
		artifacts.Publication{Mode: artifacts.Replace, Durable: true},
	); err != nil {
		return err
	}
	source := fmt.Sprintf("/proc/%d/fd/%d", os.Getpid(), own.Fd())
	_, err = system.RunNamespace(ctx, system.HostMount(), func(ctx context.Context) (struct{}, error) {
		return struct{}{}, system.Bind(ctx, source, handle)
	})
	return err
}

// Release unmounts the handle in the host and current namespaces, and removes
// the handle and owner record. Missing resources are accepted.
func (n *SavedNamespace) Release(ctx context.Context) error {
	handle := filepath.Join(n.runtime, "mount-namespace")
	_, err := system.RunNamespace(ctx, system.HostMount(), func(ctx context.Context) (struct{}, error) {
		mounted, _, err := system.MountRoot(ctx, handle)
		if err != nil || !mounted {
			return struct{}{}, err
		}
		return struct{}{}, system.Unmount(ctx, handle)
	})
	if err != nil {
		return err
	}
	mounted, _, err := system.MountRoot(ctx, handle)
	if err != nil {
		return err
	}
	if mounted {
		if err := system.Unmount(ctx, handle); err != nil {
			return err
		}
	}
	for _, path := range []string{handle, filepath.Join(n.runtime, "mount-namespace-owner.json")} {
		if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
	}
	return nil
}

// runRecoveryChild starts and joins recovery on the caller's entered namespace thread.
func runRecoveryChild(ctx context.Context, stateLock *os.File, args []string) error {
	childArgs := make([]string, 0, len(args)+1)
	for _, arg := range args {
		if arg != "--recover" {
			childArgs = append(childArgs, arg)
		}
	}
	childArgs = append(childArgs, "--recover-namespace")
	// Start and Wait stay on this entered OS thread. All of the child runtime's
	// threads inherit the mount namespace; entering only in main is too late.
	command := exec.CommandContext(ctx, "/proc/self/exe", childArgs...)
	command.ExtraFiles = []*os.File{stateLock}
	command.Stdin = os.Stdin
	command.Stdout = os.Stdout
	command.Stderr = os.Stderr
	if err := command.Run(); err != nil {
		if ctx.Err() != nil {
			return fmt.Errorf("namespace recovery: %w", ctx.Err())
		}
		var exit *exec.ExitError
		if errors.As(err, &exit) {
			status, ok := exit.Sys().(syscall.WaitStatus)
			if ok {
				//nolint:staticcheck // User-visible text, kept byte for byte.
				return fmt.Errorf("Cloud namespace recovery failed: %s", exitDescription(status))
			}
		}
		return err
	}
	return nil
}

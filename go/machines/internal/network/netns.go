//go:build linux

package network

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/wspl/demi/go/machines/internal/linux"
)

// namespaceDirectory holds a slot's network namespace file, kept alive by a bind
// of its namespace file at /run/netns/<name>, as ip netns keeps one.
const namespaceDirectory = "/run/netns"

// namespacePath returns the file that holds the namespace name.
func namespacePath(name string) string {
	return filepath.Join(namespaceDirectory, name)
}

// createNamespace creates the network namespace name; one of that name must not
// exist.
func createNamespace(name string) error {
	file := namespacePath(name)
	return linux.InNewNetworkNamespace(func() error {
		if err := os.MkdirAll(namespaceDirectory, 0o777); err != nil {
			return err
		}
		created, err := os.OpenFile(file, os.O_RDWR|os.O_CREATE|os.O_EXCL, 0o444)
		if err != nil {
			return err
		}
		if err := created.Close(); err != nil {
			return err
		}
		bound := linux.Bind("/proc/thread-self/ns/net", file)
		if bound != nil {
			// The namespace ends with this thread; the empty file is removed so a
			// later create can succeed, and a failure to remove it is reported by
			// that create.
			_ = os.Remove(file)
		}
		return bound
	})
}

// removeNamespace removes the network namespace name if it exists: its file is
// detached and deleted, and the kernel frees the namespace once nothing uses it.
func removeNamespace(name string) error {
	file := namespacePath(name)
	root, err := linux.IsMountRoot(file)
	if err != nil {
		return err
	}
	if root {
		if err := linux.Detach(file); err != nil {
			return err
		}
	}
	if err := os.Remove(file); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	return nil
}

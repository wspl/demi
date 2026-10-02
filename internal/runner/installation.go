package runner

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// instanceID names the private installation of one normalized backend URL.
func instanceID(backend runnerwire.BackendURL) string {
	return fmt.Sprintf("%x", sha256.Sum256([]byte(backend.String())))
}

// installationLease holds the OS lock until the active record is removed.
type installationLease struct {
	file   *os.File
	active string
}

// openInstallation creates and protects this backend's private state directory.
func openInstallation(ctx context.Context, root string) error {
	if err := os.MkdirAll(root, 0700); err != nil {
		return err
	}
	return process.Chmod(ctx, root, 0700)
}

// tryInstallationLock returns nil when another runner holds this installation.
func tryInstallationLock(root string) (*installationLease, error) {
	file, err := os.OpenFile(filepath.Join(root, "runner.lock"), os.O_CREATE|os.O_RDWR|os.O_APPEND, 0600)
	if err != nil {
		return nil, err
	}
	locked, err := lockInstallationFile(file)
	if err != nil || !locked {
		return nil, errors.Join(err, file.Close())
	}
	return &installationLease{file: file}, nil
}

func (l *installationLease) close() error {
	if l.file == nil {
		return nil
	}
	var err error
	if l.active != "" {
		err = os.Remove(l.active)
		if errors.Is(err, os.ErrNotExist) {
			err = nil
		}
		l.active = ""
	}
	err = errors.Join(err, l.file.Close())
	l.file = nil
	return err
}

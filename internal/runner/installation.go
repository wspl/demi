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
	if err := os.MkdirAll(root, 0o700); err != nil {
		return err
	}
	return process.Chmod(ctx, root, 0o700)
}

// errInstallationBusy reports that another runner holds the installation lock.
var errInstallationBusy = errors.New("runner already active for this installation")

// tryInstallationLock takes the installation lock, or returns errInstallationBusy when another runner holds it.
func tryInstallationLock(root string) (*installationLease, error) {
	file, err := os.OpenFile(filepath.Join(root, "runner.lock"), os.O_CREATE|os.O_RDWR|os.O_APPEND, 0o600)
	if err != nil {
		return nil, err
	}
	locked, err := lockInstallationFile(file)
	if err != nil {
		return nil, errors.Join(err, file.Close())
	}
	if !locked {
		return nil, errors.Join(errInstallationBusy, file.Close())
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

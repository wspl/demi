package artifacts

import (
	"bytes"
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"os"
)

// Mode specifies whether an artifact may replace an existing file.
type Mode int

const (
	// CreateNew fails with os.ErrExist if the destination exists.
	CreateNew Mode = iota
	// Replace atomically replaces the destination.
	Replace
)

// Permissions selects the published file's Unix permissions.
type Permissions int

const (
	// Default uses 0666 less the process umask.
	Default Permissions = iota
	// Private grants only owner read and write access.
	Private
	// Executable grants 0755 regardless of umask.
	Executable
	// Keep preserves the destination's permissions.
	Keep
)

// Publication describes how an artifact file becomes visible.
type Publication struct {
	// Mode determines whether publication may replace an existing file.
	Mode Mode
	// Permissions determines the staged file's permissions.
	Permissions Permissions
	// Durable requests file and parent-directory synchronization during publication.
	Durable bool
}

// Staged owns a temporary file. Call Close with defer immediately after creation;
// Publish closes and renames it, and Close removes any unpublished file.
type Staged struct {
	file            *os.File
	temporary, path string
	publication     Publication
}

// NewStaged creates a temporary beside path with the publication's permissions.
func NewStaged(ctx context.Context, path string, publication Publication) (*Staged, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	parent, ok := Parent(path)
	if !ok {
		return nil, fmt.Errorf("a file needs a parent directory: %w", os.ErrInvalid)
	}
	mode := os.FileMode(0o666)
	switch publication.Permissions {
	case Private, Keep:
		mode = 0o600
	case Executable:
		mode = 0o755
	}
	var file *os.File
	var err error
	for {
		// Six random characters keep the runner's recognizable partial-file name.
		name := artifactPath(parent, ".demi-partial-"+rand.Text()[:6])
		file, err = os.OpenFile(name, os.O_RDWR|os.O_CREATE|os.O_EXCL, mode)
		if !errors.Is(err, os.ErrExist) {
			break
		}
	}
	if err != nil {
		return nil, err
	}
	staged := &Staged{file, file.Name(), path, publication}
	switch publication.Permissions {
	case Keep:
		var info os.FileInfo
		info, err = os.Stat(path)
		if err == nil {
			err = file.Chmod(info.Mode())
		}
	case Executable:
		err = file.Chmod(0o755)
	}
	if err != nil {
		return nil, errors.Join(err, staged.Close())
	}
	return staged, nil
}

// File is the staged artifact's writable file, valid until Publish or Close.
func (s *Staged) File() *os.File { return s.file }

// Close discards an unpublished artifact; it is idempotent.
func (s *Staged) Close() error {
	var err error
	if s.file != nil {
		err = s.file.Close()
		s.file = nil
	}
	if s.temporary != "" {
		removeErr := os.Remove(s.temporary)
		if !errors.Is(removeErr, os.ErrNotExist) {
			err = errors.Join(err, removeErr)
		}
		s.temporary = ""
	}
	return err
}

// Publish commits a complete artifact. Cancellation is checked before commit;
// once publication starts it is completed, including requested durability.
func (s *Staged) Publish(ctx context.Context) (err error) {
	defer func() { err = errors.Join(err, s.Close()) }()
	if err := ctx.Err(); err != nil {
		return err
	}
	if s.file == nil {
		return os.ErrClosed
	}
	if s.publication.Durable {
		if err := s.file.Sync(); err != nil {
			return err
		}
	}
	err = s.file.Close()
	s.file = nil
	if err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if s.publication.Mode == CreateNew {
		if err := os.Link(s.temporary, s.path); err != nil {
			return err
		}
		if err := os.Remove(s.temporary); err != nil {
			return err
		}
	} else if err := os.Rename(s.temporary, s.path); err != nil {
		return err
	}
	s.temporary = ""
	if s.publication.Durable {
		parent, _ := Parent(s.path)
		if parent == "" {
			parent = "."
		}
		return syncDirectory(context.WithoutCancel(ctx), parent)
	}
	return nil
}

// Publish streams an artifact into an atomic publication. The caller must
// unblock input on cancellation when its reads can wait indefinitely.
func Publish(ctx context.Context, path string, input io.Reader, publication Publication) (err error) {
	staged, err := NewStaged(ctx, path, publication)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, staged.Close()) }()
	if err := transfer(ctx, input, staged.File(), nil); err != nil {
		return err
	}
	return staged.Publish(ctx)
}

// PublishBytes atomically publishes bytes at path.
func PublishBytes(ctx context.Context, path string, data []byte, publication Publication) error {
	return Publish(ctx, path, bytes.NewReader(data), publication)
}

// PublishDirectory replaces a directory with a stage on the same volume.
// Like the Rust operation, replacing a directory removes the old tree first;
// callers serialize installers with an InstallLock.
func PublishDirectory(ctx context.Context, staged, destination string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := os.RemoveAll(destination); err != nil {
		return err
	}
	return os.Rename(staged, destination)
}

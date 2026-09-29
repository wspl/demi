package artifact

import (
	"context"
	"crypto/rand"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
)

// stagedPrefix is how a staged file's name starts, before its random part: a
// process killed while it writes leaves the file behind, and the name says
// whose it is (docs/execution/runner.md § File contents).
const stagedPrefix = ".demi-partial-"

// A Mode says whether a publication may replace what is at its path.
type Mode int

// The modes of a publication.
const (
	// CreateNew fails with an error that matches [fs.ErrExist] when the path
	// exists.
	CreateNew Mode = iota
	// Replace replaces what is at the path.
	Replace
)

// Permissions are the published file's permissions, on Unix.
type Permissions int

// The permissions of a published file.
const (
	// DefaultPermissions are read and write for everyone, less the process's
	// umask: a new file's usual permissions.
	DefaultPermissions Permissions = iota
	// Private is read and write for the owner only.
	Private
	// Executable is read, write and run for the owner, read and run for
	// everyone.
	Executable
	// Keep are the permissions of the file being replaced.
	Keep
)

// A Publication says how a file is published.
type Publication struct {
	Mode        Mode
	Permissions Permissions
	// Durable says the bytes are on disk before the rename, so a crash leaves
	// the old file or the whole new one.
	Durable bool
}

// A Staged is a file being published: what the caller writes to it stays out of
// sight at its path until [Staged.Publish]. [Staged.Discard] removes it; it is
// safe to call after Publish, so a caller defers it.
type Staged struct {
	file        *os.File
	path        string
	publication Publication
}

// Stage makes a temporary file beside path with the publication's permissions.
func Stage(path string, publication Publication) (*Staged, error) {
	file, err := createStaged(path, publication)
	if err != nil {
		return nil, err
	}
	return &Staged{file: file, path: path, publication: publication}, nil
}

// File returns the staged file, for the caller to fill.
func (s *Staged) File() *os.File { return s.file }

// Publish makes the staged bytes the file at the path.
func (s *Staged) Publish() error {
	if s.file == nil {
		return errors.New("the staged file was published or discarded")
	}
	file := s.file
	s.file = nil
	name := file.Name()
	if s.publication.Durable {
		if err := file.Sync(); err != nil {
			file.Close()
			os.Remove(name)
			return err
		}
	}
	if err := file.Close(); err != nil {
		os.Remove(name)
		return err
	}
	return persist(name, s.path, s.publication.Mode)
}

// Discard removes the staged file unless it was published.
func (s *Staged) Discard() {
	if s.file == nil {
		return
	}
	name := s.file.Name()
	// The file is abandoned; a failure to close or remove it leaves a name that
	// says whose it is.
	s.file.Close()
	os.Remove(name)
	s.file = nil
}

// Publish publishes what input yields at path. Once ctx is done nothing is
// published: the rename happens only if it was not done by then. A read of input
// that blocks is not interrupted; ctx is looked at between reads.
func Publish(ctx context.Context, path string, input io.Reader, publication Publication) error {
	staged, err := Stage(path, publication)
	if err != nil {
		return err
	}
	defer staged.Discard()
	if err := copyChunks(ctx, input, staged.File(), nil); err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	return staged.Publish()
}

// PublishBytes publishes bytes at path.
func PublishBytes(path string, bytes []byte, publication Publication) error {
	staged, err := Stage(path, publication)
	if err != nil {
		return err
	}
	defer staged.Discard()
	if _, err := staged.File().Write(bytes); err != nil {
		return err
	}
	return staged.Publish()
}

// PublishDirectory moves the directory staged to destination, replacing a
// directory there. Both are on one volume, as a staging directory beside the
// destination is.
func PublishDirectory(staged, destination string) error {
	if err := os.RemoveAll(destination); err != nil {
		return err
	}
	return os.Rename(staged, destination)
}

// createStaged makes a file beside path, whose name starts with stagedPrefix,
// with the publication's permissions.
func createStaged(path string, publication Publication) (*os.File, error) {
	parent := filepath.Dir(path)
	// The mode applies when the file is made, less the umask.
	mode := fs.FileMode(0o600)
	switch publication.Permissions {
	case DefaultPermissions:
		mode = 0o666
	case Executable:
		mode = 0o755
	}
	var file *os.File
	for {
		name := filepath.Join(parent, stagedPrefix+rand.Text())
		var err error
		file, err = os.OpenFile(name, os.O_RDWR|os.O_CREATE|os.O_EXCL, mode)
		if errors.Is(err, fs.ErrExist) {
			continue
		}
		if err != nil {
			return nil, err
		}
		break
	}
	fail := func(err error) (*os.File, error) {
		file.Close()
		os.Remove(file.Name())
		return nil, err
	}
	switch publication.Permissions {
	case Keep:
		info, err := os.Stat(path)
		if err != nil {
			return fail(err)
		}
		if err := file.Chmod(info.Mode().Perm()); err != nil {
			return fail(err)
		}
	case Executable:
		// An executable is runnable whatever the umask.
		if err := file.Chmod(0o755); err != nil {
			return fail(err)
		}
	}
	return file, nil
}

// persist renames the staged file to path. Whatever happens, the staged name is
// gone.
func persist(staged, path string, mode Mode) error {
	if mode == Replace {
		if err := os.Rename(staged, path); err != nil {
			os.Remove(staged)
			return err
		}
		return nil
	}
	// A hard link fails when the path exists, which a rename would not.
	defer os.Remove(staged)
	return os.Link(staged, path)
}

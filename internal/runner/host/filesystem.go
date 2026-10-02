package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// Exists reports whether the path exists, including a dangling symlink.
func (s *Service) Exists(ctx context.Context, request runnerwire.FSExists) error {
	panic("not written: r-host")
}

// Stat reports metadata after following symlinks.
func (s *Service) Stat(ctx context.Context, request runnerwire.FSStat) error {
	panic("not written: r-host")
}

// Lstat reports metadata of the path itself without following symlinks.
func (s *Service) Lstat(ctx context.Context, request runnerwire.FSLstat) error {
	panic("not written: r-host")
}

// Readdir lists directory entries and their file types.
func (s *Service) Readdir(ctx context.Context, request runnerwire.FSReaddir) error {
	panic("not written: r-host")
}

// Mkdir creates the directory, including parents only when requested.
func (s *Service) Mkdir(ctx context.Context, request runnerwire.FSMkdir) error {
	panic("not written: r-host")
}

// Rm removes the path; directories require recursive removal and force suppresses only absence.
func (s *Service) Rm(ctx context.Context, request runnerwire.FSRm) error {
	panic("not written: r-host")
}

// Cp copies files or symlinks and, when requested, directories recursively.
func (s *Service) Cp(ctx context.Context, request runnerwire.FSCp) error {
	panic("not written: r-host")
}

// Mv renames the path, copying then removing it when devices differ.
func (s *Service) Mv(ctx context.Context, request runnerwire.FSMv) error {
	panic("not written: r-host")
}

// Chmod sets the path permissions.
func (s *Service) Chmod(ctx context.Context, request runnerwire.FSChmod) error {
	panic("not written: r-host")
}

// Symlink creates a symlink preserving the target spelling.
func (s *Service) Symlink(ctx context.Context, request runnerwire.FSSymlink) error {
	panic("not written: r-host")
}

// Link creates a hard link to the existing path.
func (s *Service) Link(ctx context.Context, request runnerwire.FSLink) error {
	panic("not written: r-host")
}

// Readlink reports the stored symlink target.
func (s *Service) Readlink(ctx context.Context, request runnerwire.FSReadlink) error {
	panic("not written: r-host")
}

// Realpath reports the canonical path.
func (s *Service) Realpath(ctx context.Context, request runnerwire.FSRealpath) error {
	panic("not written: r-host")
}

// Utimes sets access and modification times from wire millisecond timestamps.
func (s *Service) Utimes(ctx context.Context, request runnerwire.FSUtimes) error {
	panic("not written: r-host")
}

// ErrorCode returns the filesystem protocol code for an error, or nil when
// no named code applies. It recognizes wrapped operating-system errors.
func ErrorCode(err error) *string {
	panic("not written: r-host")
}

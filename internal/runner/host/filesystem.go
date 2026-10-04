package host

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/runnerproto"
)

// Exists reports whether the path exists, including a dangling symlink.
func (s *Service) Exists(ctx context.Context, request runnerproto.FSExists) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		_, err = os.Lstat(path)
		if errors.Is(err, os.ErrNotExist) {
			return &runnerproto.FSExistsResult{Value: false}, nil
		}
		return &runnerproto.FSExistsResult{Value: err == nil}, err
	})
}

// Stat reports metadata after following symlinks.
func (s *Service) Stat(ctx context.Context, request runnerproto.FSStat) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		info, err := os.Stat(path)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSStatResult{Value: fileStat(info)}, nil
	})
}

// Lstat reports metadata of the path itself without following symlinks.
func (s *Service) Lstat(ctx context.Context, request runnerproto.FSLstat) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		info, err := os.Lstat(path)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSLstatResult{Value: fileStat(info)}, nil
	})
}

// Readdir lists directory entries and their file types.
func (s *Service) Readdir(ctx context.Context, request runnerproto.FSReaddir) error {
	return s.fsCall(ctx, request.ID, func(ctx context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		entries, err := cmdsdk.Retry(ctx, func() ([]os.DirEntry, error) {
			return os.ReadDir(path)
		})
		if err != nil {
			return nil, err
		}
		result := make([]runnerproto.DirEntry, 0, len(entries))
		for _, entry := range entries {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			result = append(
				result,
				runnerproto.DirEntry{
					Name:           entry.Name(),
					IsFile:         entry.Type().IsRegular(),
					IsDirectory:    entry.IsDir(),
					IsSymbolicLink: entry.Type()&os.ModeSymlink != 0,
				},
			)
		}
		return &runnerproto.FSReaddirResult{Value: result}, nil
	})
}

// Mkdir creates the directory, including parents only when requested.
func (s *Service) Mkdir(ctx context.Context, request runnerproto.FSMkdir) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		if request.Recursive != nil && *request.Recursive {
			err = os.MkdirAll(path, 0o777)
		} else {
			err = os.Mkdir(path, 0o777)
		}
		return &runnerproto.FSMkdirResult{}, err
	})
}

// Rm removes the path; directories require recursive removal and force suppresses only absence.
func (s *Service) Rm(ctx context.Context, request runnerproto.FSRm) error {
	return s.fsCall(ctx, request.ID, func(ctx context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		err = removePath(ctx, path, request.Recursive != nil && *request.Recursive)
		if request.Force != nil && *request.Force && errors.Is(err, os.ErrNotExist) {
			err = nil
		}
		return &runnerproto.FSRmResult{}, err
	})
}

// Cp copies files or symlinks and, when requested, directories recursively.
func (s *Service) Cp(ctx context.Context, request runnerproto.FSCp) error {
	return s.fsCall(ctx, request.ID, func(ctx context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		destination, err := s.resolve(request.CWD, request.Destination)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSCpResult{}, copyPath(
			ctx,
			path,
			destination,
			request.Recursive != nil && *request.Recursive,
		)
	})
}

// Mv renames the path, copying then removing it when devices differ.
func (s *Service) Mv(ctx context.Context, request runnerproto.FSMv) error {
	return s.fsCall(ctx, request.ID, func(ctx context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		destination, err := s.resolve(request.CWD, request.Destination)
		if err != nil {
			return nil, err
		}
		err = os.Rename(path, destination)
		if errors.Is(err, syscall.EXDEV) {
			err = copyPath(ctx, path, destination, true)
			if err == nil {
				err = removePath(ctx, path, true)
			}
		}
		return &runnerproto.FSMvResult{}, err
	})
}

// Chmod sets the path permissions.
func (s *Service) Chmod(ctx context.Context, request runnerproto.FSChmod) error {
	return s.fsCall(ctx, request.ID, func(ctx context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSChmodResult{}, process.Chmod(ctx, path, request.Mode)
	})
}

// Symlink creates a symlink preserving the target spelling.
func (s *Service) Symlink(ctx context.Context, request runnerproto.FSSymlink) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSSymlinkResult{}, os.Symlink(request.Target, path)
	})
}

// Link creates a hard link to the existing path.
func (s *Service) Link(ctx context.Context, request runnerproto.FSLink) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		existing, err := s.resolve(request.CWD, request.ExistingPath)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSLinkResult{}, os.Link(existing, path)
	})
}

// Readlink reports the stored symlink target.
func (s *Service) Readlink(ctx context.Context, request runnerproto.FSReadlink) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		target, err := os.Readlink(path)
		return &runnerproto.FSReadlinkResult{Value: target}, err
	})
}

// Realpath reports the canonical path.
func (s *Service) Realpath(ctx context.Context, request runnerproto.FSRealpath) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		target, err := filepath.EvalSymlinks(path)
		if err == nil {
			target, err = filepath.Abs(target)
		}
		return &runnerproto.FSRealpathResult{Value: target}, err
	})
}

// Utimes sets access and modification times from wire millisecond timestamps.
func (s *Service) Utimes(ctx context.Context, request runnerproto.FSUtimes) error {
	return s.fsCall(ctx, request.ID, func(_ context.Context) (runnerproto.FSResult, error) {
		path, err := s.resolve(request.CWD, request.Path)
		if err != nil {
			return nil, err
		}
		return &runnerproto.FSUtimesResult{}, os.Chtimes(
			path,
			time.UnixMilli(int64(request.Atime)),
			time.UnixMilli(int64(request.Mtime)),
		)
	})
}

// ErrorCode returns the filesystem protocol code for an error, or nil when
// no named code applies. It recognizes wrapped operating-system errors.
func ErrorCode(err error) *string {
	codes := []struct {
		cause error
		code  string
	}{
		{os.ErrNotExist, "ENOENT"},
		{os.ErrPermission, "EACCES"},
		{os.ErrExist, "EEXIST"},
		{syscall.ENOTDIR, "ENOTDIR"},
		{syscall.EISDIR, "EISDIR"},
		{syscall.ENOTEMPTY, "ENOTEMPTY"},
		{os.ErrInvalid, "EINVAL"},
		{syscall.EINVAL, "EINVAL"},
		{syscall.EXDEV, "EXDEV"},
		{context.Canceled, "EINTR"},
		{syscall.EINTR, "EINTR"},
		{syscall.EROFS, "EROFS"},
		{syscall.ENOSPC, "ENOSPC"},
		{syscall.EMLINK, "EMLINK"},
		{syscall.EPIPE, "EPIPE"},
		{syscall.ELOOP, "ELOOP"},
	}
	for _, entry := range codes {
		if errors.Is(err, entry.cause) {
			return &entry.code
		}
	}
	return nil
}

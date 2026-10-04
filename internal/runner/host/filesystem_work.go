package host

import (
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/runnerproto"
)

// fsCall admits one finite filesystem request and reports its result.
func (s *Service) fsCall(
	ctx context.Context,
	id string,
	work func(context.Context) (runnerproto.FSResult, error),
) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	if err = admit(ctx, s.filesystem); err != nil {
		return err
	}
	defer func() { <-s.filesystem }()
	result, err := work(ctx)
	return s.fsReply(ctx, id, result, err)
}

// resolve selects a filesystem request's working directory before resolving its path.
func (s *Service) resolve(cwd *string, path string) (string, error) {
	base := s.defaultCWD
	if cwd != nil {
		base = *cwd
	}
	target, err := commandsdk.Resolve(base, path)
	if err != nil {
		return "", &filesystemError{message: err.Error(), cause: os.ErrInvalid}
	}
	return target, nil
}

// removePath removes the requested Host entry, requiring recursion for directories.
func removePath(ctx context.Context, path string, recursive bool) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if info.IsDir() {
		if !recursive {
			return &filesystemError{
				message: "recursive removal is required for a directory",
				cause:   syscall.EISDIR,
			}
		}
		_, err = commandsdk.Retry(ctx, func() (struct{}, error) { return struct{}{}, os.RemoveAll(path) })
		return err
	}
	return os.Remove(path)
}

// canonicalDestination resolves the existing ancestor before comparing copy boundaries.
func canonicalDestination(path string) (string, error) {
	ancestor := path
	var suffix []string
	for {
		base, err := filepath.EvalSymlinks(ancestor)
		if err == nil {
			base, err = filepath.Abs(base)
			if err != nil {
				return "", err
			}
			for i := len(suffix) - 1; i >= 0; i-- {
				base = filepath.Join(base, suffix[i])
			}
			return base, nil
		}
		if !errors.Is(err, os.ErrNotExist) {
			return "", err
		}
		parent, ok := artifacts.Parent(ancestor)
		if !ok {
			return "", &filesystemError{message: "invalid destination", cause: os.ErrInvalid}
		}
		suffix = append(suffix, filepath.Base(ancestor))
		if parent == "" {
			parent = "."
		}
		ancestor = parent
	}
}

// copyPath copies a Host entry without following its symlinks or recursing into itself.
func copyPath(ctx context.Context, source, destination string, recursive bool) error {
	info, err := os.Lstat(source)
	if err != nil {
		return err
	}
	if info.IsDir() {
		if err := checkCopyDirectory(source, destination, recursive); err != nil {
			return err
		}
	}
	return copyEntry(ctx, source, destination)
}

// pathWithin compares filesystem components, not coincidental string prefixes.
func pathWithin(path, root string) bool {
	relative, err := filepath.Rel(root, path)
	return err == nil && relative != ".." && !strings.HasPrefix(relative, ".."+string(os.PathSeparator))
}

// copyEntry preserves Host file permissions and symlink targets during a recursive copy.
func copyEntry(ctx context.Context, source, destination string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	info, err := os.Lstat(source)
	if err != nil {
		return err
	}
	if info.Mode()&os.ModeSymlink != 0 {
		target, err := os.Readlink(source)
		if err != nil {
			return err
		}
		return os.Symlink(target, destination)
	}
	if info.Mode().IsRegular() {
		_, err = commandsdk.Retry(
			ctx,
			func() (struct{}, error) { return struct{}{}, copyRegular(ctx, source, destination, info.Mode()) },
		)
		return err
	}
	if !info.IsDir() {
		return &filesystemError{message: "cannot copy a special file", cause: os.ErrInvalid}
	}
	if err = os.MkdirAll(destination, 0o777); err != nil {
		return err
	}
	entries, err := commandsdk.Retry(ctx, func() ([]os.DirEntry, error) { return os.ReadDir(source) })
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if err = copyEntry(
			ctx,
			filepath.Join(source, entry.Name()),
			filepath.Join(destination, entry.Name()),
		); err != nil {
			return err
		}
	}
	return os.Chmod(destination, info.Mode())
}

// copyRegular owns both file handles for the Host copy, including failed opens.
func copyRegular(ctx context.Context, source, destination string, mode os.FileMode) (err error) {
	in, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, in.Close()) }()
	out, err := os.OpenFile(destination, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, mode.Perm())
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, out.Close()) }()
	if _, err = io.Copy(out, &contextReader{ctx: ctx, reader: in}); err != nil {
		return err
	}
	return out.Chmod(mode)
}

// contextReader checks cancellation between Host file reads; resource owners
// still close blocking pipes and sockets when their contexts end.
type contextReader struct {
	ctx    context.Context
	reader io.Reader
}

func (r *contextReader) Read(p []byte) (int, error) {
	if err := r.ctx.Err(); err != nil {
		return 0, err
	}
	return r.reader.Read(p)
}

func checkCopyDirectory(source, destination string, recursive bool) error {
	if !recursive {
		return &filesystemError{message: "recursive copy is required for a directory", cause: syscall.EISDIR}
	}
	canonical, err := filepath.EvalSymlinks(source)
	if err != nil {
		return err
	}
	canonical, err = filepath.Abs(canonical)
	if err != nil {
		return err
	}
	dest, err := canonicalDestination(destination)
	if err != nil {
		return err
	}
	if pathWithin(dest, canonical) {
		return &filesystemError{message: "cannot copy a directory into itself", cause: os.ErrInvalid}
	}
	return nil
}

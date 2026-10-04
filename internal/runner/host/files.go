package host

import (
	"context"
	"errors"
	"io"
	"math"
	"os"
	"syscall"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/runnerproto"
)

// ReadFile opens and positions the file, replies, then streams the requested
// range to its output pipe. It reports pipe_done even when opening fails.
// Pipe failure stops the read and closes the file.
func (s *Service) ReadFile(ctx context.Context, request runnerproto.FSReadFile) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	body, failure := s.openRange(ctx, request)
	if err = s.fsReply(s.life.ctx, request.ID, &runnerproto.FSReadFileResult{}, failure); err != nil {
		if body != nil {
			_ = body.Close()
		} // The reply failure already determines the result.
		return err
	}
	if failure == nil {
		failure = s.pipes.Put(ctx, request.Output.URL, body)
	}
	return process.ReportPipe(s.life.ctx, s.output, request.Output.ID, failure)
}

// WriteFile fills an artifacts.Staged file beside the destination from the
// input pipe and publishes only after clean EOF. Failure removes the temporary
// file and preserves the destination. It reports pipe_done before the reply.
// Parent creation uses artifacts.Parent and occurs only when requested.
func (s *Service) WriteFile(ctx context.Context, request runnerproto.FSWriteFile) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	failure := s.writeFromPipe(ctx, request)
	if err = process.ReportPipe(s.life.ctx, s.output, request.Input.ID, failure); err != nil {
		return err
	}
	return s.fsReply(s.life.ctx, request.ID, &runnerproto.FSWriteFileResult{}, failure)
}

// openRange prepares a regular Host file before acknowledging its read request.
func (s *Service) openRange(ctx context.Context, request runnerproto.FSReadFile) (io.ReadCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	path, err := s.resolve(request.CWD, request.Path)
	if err != nil {
		return nil, err
	}
	file, err := cmdsdk.Retry(ctx, func() (*os.File, error) {
		return os.Open(path)
	})
	if err != nil {
		return nil, err
	}
	info, err := file.Stat()
	if err == nil && !info.Mode().IsRegular() {
		err = &filesystemError{message: "not a regular file", cause: syscall.EISDIR}
	}
	offset := uint64(0)
	if request.Offset != nil {
		offset = *request.Offset
	}
	if err == nil {
		if offset > math.MaxInt64 {
			err = syscall.EINVAL
		} else {
			_, err = file.Seek(int64(offset), io.SeekStart)
		}
	}
	if err != nil {
		return nil, errors.Join(err, file.Close())
	}
	length := uint64(math.MaxUint64)
	if request.Length != nil {
		length = *request.Length
	}
	return &fileRange{file: file, left: length}, nil
}

// fileRange retains uint64 wire ranges without narrowing their length to int64.
type fileRange struct {
	file *os.File
	left uint64
}

func (f *fileRange) Read(p []byte) (int, error) {
	if f.left == 0 {
		return 0, io.EOF
	}
	if uint64(len(p)) > f.left {
		p = p[:int(f.left)]
	}
	n, err := f.file.Read(p)
	f.left -= uint64(n)
	return n, err
}

func (f *fileRange) Close() error {
	return f.file.Close()
}

// writeFromPipe publishes only complete pipe input into the Host's destination.
func (s *Service) writeFromPipe(ctx context.Context, request runnerproto.FSWriteFile) (err error) {
	if err = ctx.Err(); err != nil {
		return err
	}
	target, err := s.resolve(request.CWD, request.Path)
	if err != nil {
		return err
	}
	parent, ok := artifacts.Parent(target)
	if !ok {
		return &filesystemError{message: "a file needs a parent directory", cause: os.ErrInvalid}
	}
	if request.CreateParents != nil && *request.CreateParents {
		if parent == "" {
			parent = "."
		}
		if err = os.MkdirAll(parent, 0o777); err != nil {
			return err
		}
	}
	staged, err := cmdsdk.Retry(ctx, func() (*artifacts.Staged, error) {
		return artifacts.NewStaged(
			ctx,
			target,
			artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Default},
		)
	})
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, staged.Close())
	}()
	body, err := s.pipes.Open(ctx, request.Input.URL)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, body.Close())
	}()
	if _, err = io.Copy(staged.File(), &contextReader{ctx: ctx, reader: body}); err != nil {
		return err
	}
	return staged.Publish(ctx)
}

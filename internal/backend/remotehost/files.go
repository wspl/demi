package remotehost

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// remoteFS implements the Host filesystem over request replies and content pipes.
type remoteFS struct{ host *Host }

// request holds admission until a filesystem reply arrives.
func (f remoteFS) request(
	ctx context.Context,
	op string,
	build func(string) runnerwire.Inbound,
) (runnerwire.FSResult, error) {
	link, err := f.host.connection()
	if err != nil {
		return nil, err
	}
	lease, err := f.host.admit()
	if err != nil {
		return nil, err
	}
	if lease != nil {
		defer lease.Release()
	}
	reply, err := link.call(ctx, fmt.Sprintf("Fs(%q)", op), build)
	if err != nil {
		return nil, err
	}
	if reply, ok := reply.(*runnerwire.FSOK); ok {
		return reply.Result, nil
	}
	return nil, mismatch()
}

// filled returns only after the runner opens the file and accepts its output pipe.
func filled(
	ctx context.Context,
	link *Link,
	expected string,
	build func(string, runnerwire.PipeRef) runnerwire.Inbound,
) (*PipeReader, error) {
	pipe := link.pipes.FromDevice(link.device)
	reader, err := pipe.Reader()
	if err != nil {
		return nil, &host.Error{Kind: host.Interrupted, Message: err.Error()}
	}
	if _, err = link.call(
		ctx,
		expected,
		func(id string) runnerwire.Inbound { return build(id, pipe.WireRef()) },
	); err != nil {
		reader.Fail(err.Error())
		return nil, err
	}
	return reader, nil
}

// collect reads a runner's pipe within the protocol's expected output bound.
func collect(ctx context.Context, reader *PipeReader, limit int) (result []byte, err error) {
	defer func() { err = errors.Join(err, reader.Close(context.WithoutCancel(ctx))) }()
	for {
		chunk, err := reader.Next(ctx)
		if errors.Is(err, io.EOF) {
			return result, nil
		}
		if err != nil {
			return nil, &host.Error{Kind: host.Interrupted, Message: err.Error()}
		}
		if len(chunk) > limit-len(result) {
			return nil, &host.Error{
				Kind:    host.Protocol,
				Message: fmt.Sprintf("the runner sent more than %d bytes", limit),
			}
		}
		result = append(result, chunk...)
	}
}

// ReadFile reads the complete file through a runner pipe.
func (f remoteFS) ReadFile(ctx context.Context, path string) ([]byte, error) {
	reader, err := f.host.ReadPipe(ctx, path, host.ByteRange{})
	if err != nil {
		return nil, err
	}
	return collect(ctx, reader, math.MaxInt)
}

// ReadStream opens a runner file pipe for the requested byte range.
func (f remoteFS) ReadStream(ctx context.Context, path string, span host.ByteRange) (host.ByteStream, error) {
	reader, err := f.host.ReadPipe(ctx, path, span)
	if err != nil {
		return nil, err
	}
	return fileStream{reader}, nil
}

// fileStream translates pipe failures into the Host filesystem error vocabulary.
type fileStream struct{ reader *PipeReader }

// Read translates pipe failures to filesystem interruptions.
func (s fileStream) Read(ctx context.Context, data []byte) (int, error) {
	n, err := s.reader.Read(ctx, data)
	if err != nil && !errors.Is(err, io.EOF) {
		return n, &host.Error{Kind: host.Interrupted, Message: err.Error()}
	}
	return n, err
}

// Close releases the file pipe reader.
func (s fileStream) Close(ctx context.Context) error { return s.reader.Close(ctx) }

// WriteFile uploads contents while the runner writes the destination file.
func (f remoteFS) WriteFile(
	ctx context.Context,
	path string,
	contents host.FileContents,
	options host.WriteOptions,
) (err error) {
	if contents.Stream != nil {
		defer func() { err = errors.Join(err, contents.Stream.Close(context.WithoutCancel(ctx))) }()
	}
	pipe, err := f.host.WritePipe()
	if err != nil {
		return err
	}
	writer, err := pipe.Writer()
	if err != nil {
		return &host.Error{Kind: host.Interrupted, Message: err.Error()}
	}
	uploadCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	uploaded := make(chan error, 1)
	go func() {
		uploadFile(uploadCtx, writer, contents, uploaded)
	}()
	written := f.host.WriteFrom(ctx, path, pipe, options)
	if written != nil {
		pipe.Fail(written.Error())
		cancel()
	}
	sourceErr := <-uploaded
	// Rust uses the source failure only when the runner refuses the write.
	// Keep that precedence while observing redundant pipe/cancellation errors.
	if sourceErr != nil {
		slog.Debug("file upload failed: "+sourceErr.Error(), "path", path)
	}
	if written == nil {
		return nil
	}
	if sourceErr != nil && !errors.Is(sourceErr, context.Canceled) {
		var failure *PipeFailure
		if !errors.As(sourceErr, &failure) {
			return sourceErr
		}
	}
	return written
}

// Exists asks the runner whether path exists.
func (f remoteFS) Exists(ctx context.Context, path string) (bool, error) {
	result, err := f.request(ctx, "exists", func(id string) runnerwire.Inbound {
		return &runnerwire.FSExists{ID: id, Path: path, CWD: new(f.host.cwd)}
	})
	if err != nil {
		return false, err
	}
	if result, ok := result.(*runnerwire.FSExistsResult); ok {
		return result.Value, nil
	}
	return false, mismatch()
}

// Stat reads metadata after following symbolic links.
func (f remoteFS) Stat(ctx context.Context, path string) (host.FileStat, error) {
	result, err := f.request(ctx, "stat", func(id string) runnerwire.Inbound {
		return &runnerwire.FSStat{ID: id, Path: path, CWD: new(f.host.cwd)}
	})
	if err != nil {
		return host.FileStat{}, err
	}
	if result, ok := result.(*runnerwire.FSStatResult); ok {
		return fileStat(result.Value)
	}
	return host.FileStat{}, mismatch()
}

// Lstat reads metadata without following symbolic links.
func (f remoteFS) Lstat(ctx context.Context, path string) (host.FileStat, error) {
	result, err := f.request(ctx, "lstat", func(id string) runnerwire.Inbound {
		return &runnerwire.FSLstat{ID: id, Path: path, CWD: new(f.host.cwd)}
	})
	if err != nil {
		return host.FileStat{}, err
	}
	if result, ok := result.(*runnerwire.FSLstatResult); ok {
		return fileStat(result.Value)
	}
	return host.FileStat{}, mismatch()
}

// ReadDir lists entries in the runner directory.
func (f remoteFS) ReadDir(ctx context.Context, path string) ([]host.DirEntry, error) {
	result, err := f.request(ctx, "readdir", func(id string) runnerwire.Inbound {
		return &runnerwire.FSReaddir{ID: id, Path: path, CWD: new(f.host.cwd)}
	})
	if err != nil {
		return nil, err
	}
	if result, ok := result.(*runnerwire.FSReaddirResult); ok {
		entries := make([]host.DirEntry, 0, len(result.Value))
		for _, entry := range result.Value {
			entries = append(
				entries,
				host.DirEntry{
					Name: entry.Name,
					Kind: fileKind(entry.IsFile, entry.IsDirectory, entry.IsSymbolicLink, nil, nil),
				},
			)
		}
		return entries, nil
	}
	return nil, mismatch()
}

// fileStat converts the runner's checked metadata to Host metadata.
func fileStat(stat runnerwire.FileStat) (host.FileStat, error) {
	modified, err := core.TimestampFromMillisecond(int64(stat.Mtime))
	if err != nil {
		return host.FileStat{}, &host.Error{Kind: host.Protocol, Message: err.Error()}
	}
	return host.FileStat{
		Kind:     fileKind(stat.IsFile, stat.IsDirectory, stat.IsSymbolicLink, stat.IsCharacterDevice, stat.IsFIFO),
		Mode:     stat.Mode,
		Size:     stat.Size,
		Modified: modified,
	}, nil
}

// fileKind preserves the runner's file-kind precedence.
func fileKind(file, directory, symlink bool, characterDevice, fifo *bool) host.FileKind {
	switch {
	case symlink:
		return host.Symlink
	case directory:
		return host.Directory
	case file:
		return host.File
	case characterDevice != nil && *characterDevice:
		return host.CharacterDevice
	case fifo != nil && *fifo:
		return host.FIFO
	default:
		return host.OtherFile
	}
}

// Mkdir creates a runner directory with the requested recursion policy.
func (f remoteFS) Mkdir(ctx context.Context, path string, options host.MkdirOptions) error {
	_, err := f.request(ctx, "mkdir", func(id string) runnerwire.Inbound {
		request := &runnerwire.FSMkdir{ID: id, CWD: new(f.host.cwd), Path: path}
		if options.Recursive {
			request.Recursive = new(true)
		}
		return request
	})
	return err
}

// Rm removes the runner path with the requested options.
func (f remoteFS) Rm(ctx context.Context, path string, options host.RmOptions) error {
	_, err := f.request(ctx, "rm", func(id string) runnerwire.Inbound {
		request := &runnerwire.FSRm{ID: id, CWD: new(f.host.cwd), Path: path}
		if options.Recursive {
			request.Recursive = new(true)
		}
		if options.Force {
			request.Force = new(true)
		}
		return request
	})
	return err
}

// Cp copies a runner path to destination.
func (f remoteFS) Cp(ctx context.Context, path, destination string, options host.CpOptions) error {
	_, err := f.request(ctx, "cp", func(id string) runnerwire.Inbound {
		request := &runnerwire.FSCp{ID: id, CWD: new(f.host.cwd), Path: path, Destination: destination}
		if options.Recursive {
			request.Recursive = new(true)
		}
		return request
	})
	return err
}

// Mv moves a runner path to destination.
func (f remoteFS) Mv(ctx context.Context, path, destination string) error {
	_, err := f.request(ctx, "mv", func(id string) runnerwire.Inbound {
		return &runnerwire.FSMv{ID: id, CWD: new(f.host.cwd), Path: path, Destination: destination}
	})
	return err
}

// Chmod changes permission bits on the runner path.
func (f remoteFS) Chmod(ctx context.Context, path string, mode uint32) error {
	_, err := f.request(ctx, "chmod", func(id string) runnerwire.Inbound {
		return &runnerwire.FSChmod{ID: id, CWD: new(f.host.cwd), Path: path, Mode: mode}
	})
	return err
}

// Symlink creates a runner symbolic link.
func (f remoteFS) Symlink(ctx context.Context, target, path string) error {
	_, err := f.request(ctx, "symlink", func(id string) runnerwire.Inbound {
		return &runnerwire.FSSymlink{ID: id, CWD: new(f.host.cwd), Path: path, Target: target}
	})
	return err
}

// Link creates a runner hard link.
func (f remoteFS) Link(ctx context.Context, existing, path string) error {
	_, err := f.request(ctx, "link", func(id string) runnerwire.Inbound {
		return &runnerwire.FSLink{ID: id, CWD: new(f.host.cwd), Path: path, ExistingPath: existing}
	})
	return err
}

// Readlink reads the runner symbolic link target.
func (f remoteFS) Readlink(ctx context.Context, path string) (string, error) {
	result, err := f.request(ctx, "readlink", func(id string) runnerwire.Inbound {
		return &runnerwire.FSReadlink{ID: id, CWD: new(f.host.cwd), Path: path}
	})
	if err != nil {
		return "", err
	}
	if result, ok := result.(*runnerwire.FSReadlinkResult); ok {
		return result.Value, nil
	}
	return "", mismatch()
}

// Realpath resolves the runner path.
func (f remoteFS) Realpath(ctx context.Context, path string) (string, error) {
	result, err := f.request(ctx, "realpath", func(id string) runnerwire.Inbound {
		return &runnerwire.FSRealpath{ID: id, CWD: new(f.host.cwd), Path: path}
	})
	if err != nil {
		return "", err
	}
	if result, ok := result.(*runnerwire.FSRealpathResult); ok {
		return result.Value, nil
	}
	return "", mismatch()
}

// Utimes sets access and modification times on the runner path.
func (f remoteFS) Utimes(ctx context.Context, path string, accessed, modified core.Timestamp) error {
	atime, err := accessed.Millisecond()
	if err != nil {
		return err
	}
	mtime, err := modified.Millisecond()
	if err != nil {
		return err
	}
	_, err = f.request(ctx, "utimes", func(id string) runnerwire.Inbound {
		return &runnerwire.FSUtimes{
			ID:    id,
			CWD:   new(f.host.cwd),
			Path:  path,
			Atime: runnerwire.Timestamp(atime),
			Mtime: runnerwire.Timestamp(mtime),
		}
	})
	return err
}

// uploadFile streams file contents and reports completion to its owning request.
func uploadFile(uploadCtx context.Context, writer *PipeWriter, contents host.FileContents, uploaded chan<- error) {
	defer writer.Fail("the writer went away before the end")
	if contents.Stream == nil {
		if err := writer.Write(uploadCtx, contents.Bytes); err != nil {
			uploaded <- err
			return
		}
		writer.End()
		uploaded <- nil
		return
	}
	buffer := make([]byte, 65536)
	for {
		n, err := contents.Stream.Read(uploadCtx, buffer)
		if n > 0 {
			if writeErr := writer.Write(uploadCtx, buffer[:n]); writeErr != nil {
				uploaded <- writeErr
				return
			}
		}
		if errors.Is(err, io.EOF) {
			writer.End()
			uploaded <- nil
			return
		}
		if err != nil {
			writer.Fail(err.Error())
			uploaded <- err
			return
		}
	}
}

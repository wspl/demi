package shell

import (
	"context"
	"io"
	"io/fs"
	"os"
	"strconv"

	"github.com/wspl/demi/go/commandservice"
	"mvdan.cc/sh/v3/interp"
)

// open keeps redirection writes inside the job's recorder. Host account
// permissions are the boundary: cwd is not a filesystem sandbox.
func (s *scope) open(ctx context.Context, path string, flag int, mode os.FileMode) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	var file io.ReadWriteCloser
	operation := func() error {
		var err error
		if mask := interp.HandlerCtx(ctx).Env.Get(maskVariable).String(); mask != "" {
			value, failure := strconv.ParseUint(mask, 8, 32)
			if failure != nil {
				return failure
			}
			mode &^= os.FileMode(value)
		}
		file, err = commandservice.RetryBlocking(func() (io.ReadWriteCloser, error) {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			return interp.DefaultOpenHandler()(ctx, path, flag, mode)
		})
		return err
	}
	// The null device keeps the name the interpreter's opener knows on every
	// system; any other path converts a Windows drive path first, as cd and
	// program lookup do (the Rust's scope.rs resolve_path).
	if path != "/dev/null" {
		resolved, err := commandservice.ResolvePath(interp.HandlerCtx(ctx).Dir, interp.DrivePath(path))
		if err != nil {
			return nil, err
		}
		path = resolved
	}
	writing := flag&(os.O_WRONLY|os.O_RDWR|os.O_TRUNC|os.O_APPEND) != 0
	if s.options.Recorder == nil || !writing {
		err := operation()
		return file, err
	}
	if err := s.options.Recorder.Record(path, operation); err != nil {
		if file != nil {
			file.Close()
		}
		return nil, err
	}
	return &recordedFile{ReadWriteCloser: file, recorder: s.options.Recorder, path: path}, nil
}

func (s *scope) stat(ctx context.Context, path string, follow bool) (fs.FileInfo, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return interp.DefaultStatHandler()(ctx, path, follow)
}

type recordedFile struct {
	io.ReadWriteCloser
	recorder *commandservice.Recorder
	path     string
}

func (f *recordedFile) Write(p []byte) (int, error) {
	var n int
	// A path replaced while this descriptor is open names a different file.
	if original := f.FileHandle(); original != nil {
		held, heldErr := original.Stat()
		current, currentErr := os.Stat(f.path)
		if heldErr != nil || currentErr != nil || !os.SameFile(held, current) {
			return f.ReadWriteCloser.Write(p)
		}
	}
	err := f.recorder.Record(f.path, func() error {
		var err error
		n, err = f.ReadWriteCloser.Write(p)
		return err
	})
	return n, err
}

// FileHandle preserves native descriptor semantics for input and extra child
// descriptors. External writes through descriptors above 2 are outside tracking
// (edit-tracking.md); stdout and stderr are forwarded through Write instead.
func (f *recordedFile) FileHandle() *os.File {
	if file, ok := f.ReadWriteCloser.(*os.File); ok {
		return file
	}
	if wrapped, ok := f.ReadWriteCloser.(interface{ FileHandle() *os.File }); ok {
		return wrapped.FileHandle()
	}
	return nil
}

// pipe applies the SDK's descriptor-pressure policy to interpreter pipes.
func (s *scope) pipe(ctx context.Context) (*os.File, *os.File, error) {
	type ends struct{ reader, writer *os.File }
	pair, err := commandservice.RetryBlocking(func() (ends, error) {
		if err := ctx.Err(); err != nil {
			return ends{}, err
		}
		reader, writer, err := os.Pipe()
		return ends{reader, writer}, err
	})
	return pair.reader, pair.writer, err
}

// retryIO connects the interpreter's internal allocations to the SDK policy.
func retryIO(ctx context.Context, attempt func() error) error {
	_, err := commandservice.RetryBlocking(func() (struct{}, error) {
		if err := ctx.Err(); err != nil {
			return struct{}{}, err
		}
		return struct{}{}, attempt()
	})
	return err
}

// openWaiting opens name for reading, waiting out a lack of descriptors until
// ctx ends (runner.md § Load): a missing file fails at once, a full descriptor
// table does not.
func openWaiting(ctx context.Context, name string) (*os.File, error) {
	return commandservice.RetryBlocking(func() (*os.File, error) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		return os.Open(name)
	})
}

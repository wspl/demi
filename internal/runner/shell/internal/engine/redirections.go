package engine

import (
	"context"
	"io"
	"os"

	"github.com/wspl/demi/internal/cmdsdk"
	"mvdan.cc/sh/v3/interp"
)

// open captures truncation at open and wraps subsequent writes as separate mutations.
func (e *execution) open(ctx context.Context, path string, flags int, mode os.FileMode) (io.ReadWriteCloser, error) {
	handler := interp.HandlerCtx(ctx)
	absolute, err := cmdsdk.Resolve(handler.Dir, shellPath(path))
	if err != nil {
		return nil, err
	}
	state := handler.Scope().(*interpreterScope)
	if state.attributes.Umask != nil {
		mode &^= os.FileMode(*state.attributes.Umask)
	}
	writing := flags&(os.O_WRONLY|os.O_RDWR) != 0
	recording := e.record(ctx, absolute, writing)
	if recording != nil {
		defer recording.Close(ctx)
	}
	file, err := cmdsdk.Retry(ctx, func() (*os.File, error) {
		return os.OpenFile(absolute, flags, mode)
	})
	if err != nil {
		return nil, err
	}
	var result io.ReadWriteCloser = file
	if writing && e.options.Edits != nil {
		info, statErr := file.Stat()
		if statErr == nil && info.Mode().IsRegular() {
			result = &recordedFile{file: file, path: absolute, owner: e, ctx: ctx, readWrite: flags&os.O_RDWR != 0}
		}
	}
	e.mu.Lock()
	e.files = append(e.files, result)
	e.mu.Unlock()
	return result, nil
}

// record starts a job mutation only for regular files or new paths.
func (e *execution) record(ctx context.Context, path string, writing bool) *cmdsdk.Recording {
	if !writing || e.options.Edits == nil {
		return nil
	}
	if info, err := os.Stat(path); err == nil && !info.Mode().IsRegular() {
		return nil
	}
	recording := e.options.Edits.Begin(ctx)
	if recording == nil {
		return nil
	} // Recording is diagnostic and must not fail the mutation.
	recording.Track(ctx, path)
	return recording
}

type recordedFile struct {
	file      *os.File
	path      string
	owner     *execution
	ctx       context.Context
	readWrite bool
}

// Read preserves the native descriptor position for shell reads.
func (f *recordedFile) Read(b []byte) (int, error) {
	return f.file.Read(b)
}

// Close releases the retained native handle.
func (f *recordedFile) Close() error {
	return f.file.Close()
}

// Write records a mutation only while the path still identifies the opened file.
func (f *recordedFile) Write(b []byte) (int, error) {
	current, err := os.Stat(f.path)
	original, statErr := f.file.Stat()
	if err == nil && statErr == nil && os.SameFile(current, original) {
		recording := f.owner.record(f.ctx, f.path, true)
		if recording != nil {
			defer recording.Close(f.ctx)
		}
	}
	return f.file.Write(b)
}

// pipe allocates shell pipes through the runner's shared descriptor retry.
func (e *execution) pipe(ctx context.Context) (io.ReadCloser, io.WriteCloser, error) {
	type ends struct{ reader, writer *os.File }
	pair, err := cmdsdk.Retry(ctx, func() (ends, error) {
		reader, writer, err := os.Pipe()
		return ends{reader, writer}, err
	})
	return pair.reader, pair.writer, err
}

package interp

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"sync"
)

// Descriptor retains a redirection's readable and writable sides.
type Descriptor struct {
	Reader io.Reader
	Writer io.Writer
}

func (r *Runner) descriptor(fd int) Descriptor {
	switch fd {
	case 0:
		return r.standard(fd, Descriptor{Reader: r.stdin})
	case 1:
		return r.standard(fd, Descriptor{Writer: r.stdout})
	case 2:
		return r.standard(fd, Descriptor{Writer: r.stderr})
	default:
		return r.fds[fd]
	}
}

// standard returns a standard descriptor's sides. Its stream is one side; a
// read/write redirection keeps both sides in fds, and they hold while their
// file is still the stream's (a pipeline or another redirection replaces the
// stream without touching fds).
func (r *Runner) standard(fd int, current Descriptor) Descriptor {
	both, ok := r.fds[fd]
	if !ok {
		return current
	}
	if fd == 0 {
		if file := descriptorFile(current.Reader); file != nil && file == descriptorFile(both.Reader) {
			return Descriptor{Reader: current.Reader, Writer: both.Writer}
		}
		return current
	}
	if file := descriptorFile(current.Writer); file != nil && file == descriptorFile(both.Writer) {
		return Descriptor{Reader: both.Reader, Writer: current.Writer}
	}
	return current
}

func (r *Runner) setDescriptor(fd int, value Descriptor) error {
	if fd <= 2 {
		if value.Reader != nil && value.Writer != nil {
			if r.fds == nil {
				r.fds = make(map[int]Descriptor)
			}
			r.fds[fd] = value
		} else {
			delete(r.fds, fd)
		}
	}
	switch fd {
	case 0:
		if value.Reader == nil {
			r.stdin = nil
			return nil
		}
		file, err := newStdinFile(value.Reader)
		if err != nil {
			return err
		}
		r.stdin = file
	case 1:
		r.stdout = value.Writer
		if r.stdout == nil {
			r.stdout = io.Discard
		}
	case 2:
		r.stderr = value.Writer
		if r.stderr == nil {
			r.stderr = io.Discard
		}
	default:
		if r.fds == nil {
			r.fds = make(map[int]Descriptor)
		}
		r.fds[fd] = value
	}
	return nil
}

// CommandFiles connects extra descriptors to a child. Call the returned cleanup
// after the command has finished (also after a failed start). Real files retain
// their seek position; writer hooks receive bytes through a pipe.
func (hc HandlerContext) CommandFiles(ctx context.Context, cmd *exec.Cmd) (func(), error) {
	var files []*os.File
	var workers sync.WaitGroup
	var stops []func() bool
	cleanup := func() {
		for _, file := range files {
			file.Close()
		}
		workers.Wait()
		for _, stop := range stops {
			stop()
		}
	}
	for fd, value := range hc.runner.fds {
		if fd < 3 {
			continue
		}
		for len(cmd.ExtraFiles) <= fd-3 {
			cmd.ExtraFiles = append(cmd.ExtraFiles, nil)
		}
		var underlying any = value.Writer
		if underlying == nil {
			underlying = value.Reader
		}
		if unwrapped, ok := underlying.(interface{ FileHandle() *os.File }); ok {
			underlying = unwrapped.FileHandle()
		}
		if file, ok := underlying.(*os.File); ok {
			cmd.ExtraFiles[fd-3] = file
			continue
		}
		if underlying == nil {
			continue
		}
		if value.Reader != nil && value.Writer != nil {
			cleanup()
			return nil, errors.New("non-file read/write descriptor")
		}
		reader, writer, err := pipeContext(ctx)
		if err != nil {
			cleanup()
			return nil, err
		}
		workers.Add(1)
		if value.Writer != nil {
			cmd.ExtraFiles[fd-3] = writer
			files = append(files, writer)
			stops = append(stops, context.AfterFunc(ctx, func() { reader.Close() }))
			go func() { defer workers.Done(); defer reader.Close(); io.Copy(value.Writer, reader) }()
		} else {
			cmd.ExtraFiles[fd-3] = reader
			files = append(files, reader)
			stops = append(stops, context.AfterFunc(ctx, func() { writer.Close() }))
			go func() { defer workers.Done(); defer writer.Close(); io.Copy(writer, value.Reader) }()
		}
	}
	return cleanup, nil
}

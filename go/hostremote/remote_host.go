package hostremote

import (
	"context"
	"fmt"
	"io"
	"sync"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

// Executor runs a short state change on the owning shard. Its closure never
// waits; callers invoke Host operations from their owned worker goroutines.
type Executor func(context.Context, func()) error

type DeviceLink struct {
	Link *Link
	Last *shell.HostIdentity
}

// Admission returns a release function for the work's lease, or refuses it.
// Both admission and release run through the supplied shard executor.
type Admission func() (func(), error)
type RemoteHost struct {
	key       shell.HostKey
	cwd       string
	execute   Executor
	device    func() DeviceLink
	admission Admission
}

func NewRemoteHost(key shell.HostKey, cwd string, execute Executor, device func() DeviceLink, admission Admission) *RemoteHost {
	return &RemoteHost{key: key, cwd: cwd, execute: execute, device: device, admission: admission}
}
func (h *RemoteHost) Key() shell.HostKey         { return h.key }
func (h *RemoteHost) DefaultCwd() string         { return h.cwd }
func (h *RemoteHost) FS() shell.HostFS           { return h }
func (h *RemoteHost) Process() shell.HostProcess { return h }
func (h *RemoteHost) Identity() shell.HostIdentity {
	identity := shell.HostIdentity{HomeDir: h.cwd}
	// Identity has no failure in the Host contract. A retired shard leaves the
	// fallback identity, just as an offline Host without a previous hello does.
	_ = h.execute(context.Background(), func() {
		d := h.device()
		if d.Link != nil {
			identity = d.Link.Identity()
		} else if d.Last != nil {
			identity = *d.Last
		}
	})
	return identity
}
func (h *RemoteHost) link(ctx context.Context) (*Link, error) {
	var link *Link
	if err := h.execute(ctx, func() { link = h.device().Link }); err != nil {
		return nil, err
	}
	if link == nil {
		return nil, &shell.HostError{Kind: shell.HostOffline, Message: "runner disconnected"}
	}
	if link.IsClosed() {
		return nil, link.offline()
	}
	return link, nil
}
func (h *RemoteHost) Online(ctx context.Context) bool {
	_, err := h.link(ctx)
	return err == nil
}
func (h *RemoteHost) admit(ctx context.Context) (func(), error) {
	if h.admission == nil {
		return func() {}, nil
	}
	var release func()
	var refusal error
	if err := h.execute(ctx, func() { release, refusal = h.admission() }); err != nil {
		return nil, err
	}
	if refusal != nil {
		return nil, refusal
	}
	var once sync.Once
	return func() {
		once.Do(func() {
			// A retired shard has already released its state. There is no lease to
			// mutate after its executor refuses shutdown cleanup.
			_ = h.execute(context.Background(), release)
		})
	}, nil
}
func (h *RemoteHost) fs(ctx context.Context, op string, makeRequest func(string) runnerproto.Inbound) (runnerproto.FSResult, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	release, err := h.admit(ctx)
	if err != nil {
		return nil, err
	}
	defer release()
	value, err := link.call(ctx, `Fs("`+op+`")`, makeRequest)
	if err != nil {
		return nil, err
	}
	result, ok := value.(runnerproto.FSResult)
	if !ok {
		return nil, protocolError("the runner answered another request")
	}
	return result, nil
}
func (h *RemoteHost) ReadPipe(ctx context.Context, path string, span shell.ByteRange) (*PipeReader, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	release, err := h.admit(ctx)
	if err != nil {
		return nil, err
	}
	defer release()
	return filled(ctx, link, `Fs("readFile")`, func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		var offset *uint64
		if span.Offset > 0 {
			offset = &span.Offset
		}
		return runnerproto.InboundFSReadFile{ID: id, Path: path, Cwd: &h.cwd, Offset: offset, Length: span.Length, Output: output}
	})
}
func (h *RemoteHost) ReadStream(ctx context.Context, path string, span shell.ByteRange) (io.ReadCloser, error) {
	reader, err := h.ReadPipe(ctx, path, span)
	if err != nil {
		return nil, err
	}
	return reader, nil
}
func (h *RemoteHost) ReadFile(ctx context.Context, path string) ([]byte, error) {
	reader, err := h.ReadPipe(ctx, path, shell.ByteRange{})
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	return collectPipe(ctx, reader, 0)
}
func (h *RemoteHost) WritePipe(ctx context.Context) (*Pipe, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	return link.pipes.ToDevice(link.device), nil
}
func (h *RemoteHost) WriteFrom(ctx context.Context, path string, input *Pipe, options shell.WriteOptions) error {
	_, err := h.fs(ctx, "writeFile", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSWriteFile{ID: id, Path: path, Cwd: &h.cwd, CreateParents: truePointer(options.CreateParents), Input: input.WireRef()}
	})
	return err
}
func (h *RemoteHost) WriteFile(ctx context.Context, path string, contents io.ReadCloser, options shell.WriteOptions) error {
	defer contents.Close()
	pipe, err := h.WritePipe(ctx)
	if err != nil {
		return err
	}
	writer, err := pipe.Writer()
	if err != nil {
		return err
	}
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(stopped)
		pipe.Fail("write cancelled")
		contents.Close()
	})
	defer func() {
		if !stop() {
			<-stopped
		}
	}()
	source := make(chan error, 1)
	go func() {
		// Only a failure from the input itself outranks the runner's reply.
		// Closing a quiet source after refusal is teardown, not an input failure.
		buffer := make([]byte, 65536)
		for {
			n, readError := contents.Read(buffer)
			if n > 0 {
				if _, err := writer.WriteContext(ctx, buffer[:n]); err != nil {
					writer.Abort(err.Error())
					source <- nil
					return
				}
			}
			if readError != nil {
				if readError != io.EOF && pipe.Failure() == nil {
					writer.Abort(readError.Error())
					source <- readError
				} else {
					writer.Close()
					source <- nil
				}
				return
			}
		}
	}()
	written := h.WriteFrom(ctx, path, pipe, options)
	if written != nil {
		pipe.Fail(written.Error())
		contents.Close()
	}
	if sourceError := <-source; sourceError != nil {
		return sourceError
	}
	return written
}

// truePointer omits false optional runner flags, as the Rust wire does.
func truePointer(value bool) *bool {
	if value {
		return &value
	}
	return nil
}
func filled(ctx context.Context, link *Link, expected string, message func(string, runnerproto.PipeRef) runnerproto.Inbound) (*PipeReader, error) {
	pipe := link.pipes.FromDevice(link.device)
	reader, err := pipe.Reader()
	if err != nil {
		return nil, err
	}
	if _, err = link.call(ctx, expected, func(id string) runnerproto.Inbound { return message(id, pipe.WireRef()) }); err != nil {
		pipe.Fail(err.Error())
		reader.Close()
		return nil, err
	}
	return reader, nil
}

// collectPipe collects a Host transfer; zero means no additional byte bound.
func collectPipe(ctx context.Context, reader *PipeReader, limit int) ([]byte, error) {
	var bytes []byte
	for {
		chunk, err := reader.Next(ctx)
		if err == io.EOF {
			return bytes, nil
		}
		if err != nil {
			return nil, &shell.HostError{Kind: shell.HostInterrupted, Message: err.Error()}
		}
		if limit > 0 && len(bytes)+len(chunk) > limit {
			return nil, protocolError(fmt.Sprintf("the runner sent more than %d bytes", limit))
		}
		bytes = append(bytes, chunk...)
	}
}
func fileKind(stat runnerproto.FileStat) shell.FileKind {
	switch {
	case stat.IsSymbolicLink:
		return shell.FileSymlink
	case stat.IsDirectory:
		return shell.FileDirectory
	case stat.IsFile:
		return shell.FileRegular
	case stat.IsCharacterDevice != nil && *stat.IsCharacterDevice:
		return shell.FileCharacterDevice
	case stat.IsFIFO != nil && *stat.IsFIFO:
		return shell.FileFIFO
	default:
		return shell.FileOther
	}
}
func fileStat(stat runnerproto.FileStat) (shell.FileStat, error) {
	modified, err := core.TimestampFromMillisecond(stat.Mtime)
	if err != nil {
		return shell.FileStat{}, protocolError(err.Error())
	}
	return shell.FileStat{Kind: fileKind(stat), Mode: stat.Mode, Size: stat.Size, Modified: modified}, nil
}

// JobStart pins the commands and identity carried by a single shell execution.
type JobStart struct {
	Script, Cwd   string
	Env           map[string]string
	Context       commandservice.CommandContext
	Caller        *shell.JobCaller
	Commands      *CommandSelection
	Stdin, Stdout *runnerproto.PipeRef
}

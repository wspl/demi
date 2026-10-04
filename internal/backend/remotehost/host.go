package remotehost

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
)

// DeviceLink is a device's current connection, or its last identity while offline.
// A nil Link means offline; Last is used only while offline.
type DeviceLink struct {
	// Link is the current runner connection, if any.
	Link *Link
	// Last is the identity retained after the runner disconnects.
	Last *host.Identity
}

// DeviceSourceFunc returns an atomic, immutable snapshot of a device's connection.
// The device owner synchronizes publication; a Host reloads it for each operation.
type DeviceSourceFunc func() DeviceLink

// Admission takes a Cloud admission lease without waiting, or refuses work.
// Nil means free admission, as on a paired device. The operation releases the lease.
type Admission func() (*gates.Lease, error)

// Host is a host.Host over a runner connection, including its job and service facets.
type Host struct {
	key       host.Key
	cwd       string
	source    DeviceSourceFunc
	admission Admission
}

// NewHost constructs a Host against the device owner's connection snapshots.
func NewHost(key host.Key, defaultCWD string, source DeviceSourceFunc, admission Admission) *Host {
	return &Host{key: key, cwd: defaultCWD, source: source, admission: admission}
}

// Online reports whether the device's runner is connected.
func (h *Host) Online() bool {
	_, err := h.connection()
	return err == nil
}

// Key returns the execution target's value identity.
func (h *Host) Key() host.Key {
	return h.key
}

// DefaultCWD returns the base for relative paths.
func (h *Host) DefaultCWD() string {
	return h.cwd
}

// Identity returns the runner's current or last reported account.
func (h *Host) Identity() host.Identity {
	device := h.source()
	if device.Link != nil {
		return device.Link.Identity()
	}
	if device.Last != nil {
		return *device.Last
	}
	return host.Identity{HomeDir: h.cwd}
}

// FS returns the Host's filesystem facet.
func (h *Host) FS() host.FS {
	return remoteFS{host: h}
}

// Process returns the Host's process facet.
func (h *Host) Process() host.Process {
	return processFacet{host: h}
}

// StartJob starts one shell job. Offline jobs are already ended; admission may refuse them.
func (h *Host) StartJob(ctx context.Context, job JobStart) (*Job, error) {
	lease, err := h.admit()
	if err != nil {
		return nil, err
	}
	jobCtx, cancel := context.WithCancel(context.Background())
	running := &Job{
		id:       rand.Text(),
		state:    newJobState[JobEnd, JobOutput](),
		ctx:      jobCtx,
		cancel:   cancel,
		lease:    lease,
		origin:   JobOrigin{Host: h.key, Context: job.Context, Caller: job.Caller},
		commands: job.Commands,
	}
	link, err := h.connection()
	if err != nil {
		running.finish(JobEnd{Status: host.ProcessEnd{Kind: host.ProcessLost, Reason: "runner disconnected"}})
		return running, nil
	}
	running.link = link
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		running.finish(JobEnd{Status: host.ProcessEnd{Kind: host.ProcessLost, Reason: "runner disconnected"}})
		return running, nil
	}
	link.jobs[running.id] = running
	link.mu.Unlock()
	if err := running.start(ctx, job); err != nil {
		link.mu.Lock()
		delete(link.jobs, running.id)
		link.mu.Unlock()
		running.finish(JobEnd{Status: host.ProcessEnd{Kind: host.ProcessLost, Reason: err.Error()}})
		return nil, err
	}
	return running, nil
}

// ReadPipe opens the file before returning a pipe reader. Closing it stops the read.
func (h *Host) ReadPipe(ctx context.Context, path string, span host.ByteRange) (*PipeReader, error) {
	link, err := h.connection()
	if err != nil {
		return nil, err
	}
	lease, err := h.admit()
	if err != nil {
		return nil, err
	}
	if lease != nil {
		defer lease.Release()
	}
	return filled(ctx, link, `Fs("readFile")`, func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		request := &runnerproto.FSReadFile{ID: id, Path: path, CWD: new(h.cwd), Length: span.Length, Output: output}
		if span.Offset > 0 {
			request.Offset = new(span.Offset)
		}
		return request
	})
}

// WritePipe creates a pipe to the runner that this process fills.
func (h *Host) WritePipe() (*Pipe, error) {
	link, err := h.connection()
	if err != nil {
		return nil, err
	}
	return link.pipes.ToDevice(link.device), nil
}

// WriteFrom atomically replaces a file after input ends cleanly.
func (h *Host) WriteFrom(ctx context.Context, path string, input *Pipe, options host.WriteOptions) error {
	link, err := h.connection()
	if err != nil {
		return err
	}
	lease, err := h.admit()
	if err != nil {
		return err
	}
	if lease != nil {
		defer lease.Release()
	}
	_, err = link.call(ctx, `Fs("writeFile")`, func(id string) runnerproto.Inbound {
		request := &runnerproto.FSWriteFile{ID: id, Path: path, CWD: new(h.cwd), Input: input.WireRef()}
		if options.CreateParents {
			request.CreateParents = new(true)
		}
		return request
	})
	return err
}

// GitChanges reads the working-tree changes beneath root.
func (h *Host) GitChanges(ctx context.Context, root string) (runnerproto.GitChanges, error) {
	link, err := h.connection()
	if err != nil {
		return runnerproto.GitChanges{}, err
	}
	lease, err := h.admit()
	if err != nil {
		return runnerproto.GitChanges{}, err
	}
	if lease != nil {
		defer lease.Release()
	}
	reply, err := link.call(
		ctx,
		`Git("changes")`,
		func(id string) runnerproto.Inbound {
			return &runnerproto.GitChangesMessage{ID: id, Root: root}
		},
	)
	if err != nil {
		return runnerproto.GitChanges{}, err
	}
	if reply, ok := reply.(*runnerproto.GitOK); ok {
		if result, ok := reply.Result.(*runnerproto.GitChangesResult); ok {
			return result.Value, nil
		}
	}
	return runnerproto.GitChanges{}, mismatch()
}

// GitShow reads a file's committed contents from root.
func (h *Host) GitShow(ctx context.Context, root, path string) ([]byte, error) {
	link, err := h.connection()
	if err != nil {
		return nil, err
	}
	lease, err := h.admit()
	if err != nil {
		return nil, err
	}
	if lease != nil {
		defer lease.Release()
	}
	reader, err := filled(ctx, link, `Git("show")`, func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		return &runnerproto.GitShow{ID: id, Root: root, Path: path, Output: output}
	})
	if err != nil {
		return nil, err
	}
	return collect(ctx, reader, math.MaxInt)
}

// ReadLog reads up to limit lines after since, oldest first, optionally from one source.
func (h *Host) ReadLog(ctx context.Context, since *uint64, limit uint64, source *string) (LogPage, error) {
	link, err := h.connection()
	if err != nil {
		return LogPage{}, err
	}
	reply, err := link.call(ctx, "Log", func(id string) runnerproto.Inbound {
		return &runnerproto.LogRead{ID: id, Since: since, Limit: limit, Source: source}
	})
	if err != nil {
		return LogPage{}, err
	}
	if reply, ok := reply.(*runnerproto.LogLines); ok {
		return LogPage{Lines: reply.Lines, Next: reply.Next}, nil
	}
	return LogPage{}, mismatch()
}

// OpenNet connects a TCP stream between the socket and two pipes.
func (h *Host) OpenNet(ctx context.Context, hostname string, port uint16, input, output runnerproto.PipeRef) error {
	link, err := h.connection()
	if err != nil {
		return err
	}
	lease, err := h.admit()
	if err != nil {
		return err
	}
	if lease != nil {
		defer lease.Release()
	}
	_, err = link.call(ctx, "Net", func(id string) runnerproto.Inbound {
		return &runnerproto.NetOpen{StreamID: id, Host: hostname, Port: port, Input: input, Output: output}
	})
	return err
}

// OpenService opens a resident user stream. Streams are retention, not activity.
// The caller must Close the returned stream, including after Done.
func (h *Host) OpenService(
	ctx context.Context,
	request ServiceRequest,
	input, output runnerproto.PipeRef,
) (*ServiceStream, error) {
	link, err := h.connection()
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancel(link.ctx)
	stream := &ServiceStream{
		link:    link,
		id:      rand.Text(),
		request: request,
		ctx:     lifetime,
		cancel:  cancel,
		done:    make(chan struct{}),
	}
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		cancel()
		return nil, link.offline()
	}
	link.services[stream.id] = stream
	link.mu.Unlock()
	_, err = link.callID(ctx, stream.id, "Service", func(id string) runnerproto.Inbound {
		message := &runnerproto.ServiceOpen{
			StreamID:  id,
			Context:   request.Context,
			Package:   request.Package,
			Operation: request.Operation,
			JSON:      request.JSON,
			CWD:       request.CWD,
			Input:     input,
			Output:    output,
		}
		if request.Args != nil {
			message.Args = new(request.Args)
		}
		return message
	})
	if err != nil {
		stream.Close()
		return nil, err
	}
	return stream, nil
}

// CallService sends one complete input and returns at most maxBytes of output.
// A nonzero exit reports its code, stderr tail and whole stdout in a *ServiceExitError.
func (h *Host) CallService(ctx context.Context, request ServiceRequest, input []byte, maxBytes int) ([]byte, error) {
	link, err := h.connection()
	if err != nil {
		return nil, err
	}
	incoming := link.pipes.ToDevice(link.device)
	outgoing := link.pipes.FromDevice(link.device)
	writer, err := incoming.Writer()
	if err != nil {
		return nil, err
	}
	defer writer.Fail("the writer went away before the end")
	reader, err := outgoing.Reader()
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeErr := reader.Close(context.WithoutCancel(ctx)); closeErr != nil {
			slog.Debug("service reader close failed", "error", closeErr)
		}
	}()
	stream, err := h.OpenService(ctx, request, incoming.WireRef(), outgoing.WireRef())
	if err != nil {
		incoming.Fail("service call failed")
		outgoing.Fail("service call failed")
		return nil, err
	}
	defer stream.Close()
	if err = writer.Write(ctx, input); err != nil {
		incoming.Fail("service call failed")
		outgoing.Fail("service call failed")
		return nil, &host.Error{Kind: host.Interrupted, Message: err.Error()}
	}
	writer.End()
	output, err := collectService(ctx, reader, incoming, outgoing, maxBytes)
	if err != nil {
		return nil, err
	}
	end, err := stream.Done(ctx)
	if err != nil {
		return nil, err
	}
	if end.ExitCode != 0 {
		return nil, &ServiceExitError{ExitCode: end.ExitCode, Stderr: end.Stderr, Stdout: output}
	}
	return output, nil
}

// ReleaseConversation retires the conversation's resident services.
func (h *Host) ReleaseConversation(ctx context.Context, conversation string) error {
	link, err := h.connection()
	if err != nil {
		return nil
	}
	return link.ReleaseConversation(ctx, conversation)
}

// HostIdentity converts the runner account to the Host's identity.
func HostIdentity(identity runnerproto.HostIdentity) host.Identity {
	return host.Identity{UID: identity.UID, GID: identity.GID, Hostname: identity.Hostname, HomeDir: identity.HomeDir}
}

// JobStart describes one shell job and its optional declared commands and pipes.
type JobStart struct {
	// Script is the shell source dispatched to the runner.
	Script string
	// CWD is the starting working directory.
	CWD string
	// Env is exactly the variables above the device's environment.
	Env map[string]string
	// Context identifies the conversation and command locale.
	Context cmdproto.Context
	// Caller identifies the node that owns command callbacks.
	Caller *host.JobCaller
	// Commands pins the declared command manifest for this job.
	Commands *CommandSelection
	// Stdin names the optional input pipe.
	Stdin *runnerproto.PipeRef
	// Stdout names the optional output pipe.
	Stdout *runnerproto.PipeRef
}

// Job is one shell job on the runner. The owner releases it after keeping its output and edits.
type Job struct {
	id       string
	link     *Link
	state    *jobState[JobEnd, JobOutput]
	ctx      context.Context
	cancel   context.CancelFunc
	origin   JobOrigin
	commands *CommandSelection
	lease    *gates.Lease
	finished sync.Once
}

// ID returns the runner's job identifier.
func (j *Job) ID() string {
	return j.id
}

// NextOutput returns a view, or io.EOF after the job ends and all views are taken.
func (j *Job) NextOutput(ctx context.Context) (JobOutput, error) {
	return j.state.next(ctx)
}

// Follow starts or stops output beyond each stream's initial view; ended jobs do nothing.
func (j *Job) Follow(ctx context.Context, follow bool) error {
	if !j.live() {
		return nil
	}
	return j.link.send(ctx, &runnerproto.JobFollow{JobID: j.id, Follow: follow})
}

// RunningHint returns guidance from the latest declared command that supplies it.
func (j *Job) RunningHint() *string {
	return j.state.runningHint()
}

// End waits for the job's terminal result.
func (j *Job) End(ctx context.Context) (JobEnd, error) {
	return j.state.wait(ctx)
}

// WriteStdin writes ordered frames within the runner's stdin chunk limit.
func (j *Job) WriteStdin(ctx context.Context, bytes []byte) error {
	if !j.live() {
		return nil
	}
	return sendStdin(
		ctx,
		j.link,
		bytes,
		func(chunk []byte) runnerproto.Inbound {
			return &runnerproto.JobStdin{JobID: j.id, Bytes: chunk}
		},
	)
}

// CloseStdin ends the job's standard input; ended jobs do nothing.
func (j *Job) CloseStdin(ctx context.Context) error {
	if !j.live() {
		return nil
	}
	return j.link.send(ctx, &runnerproto.JobStdinEnd{JobID: j.id})
}

// ReadOutput reads the kept output while running or ended, until release.
func (j *Job) ReadOutput(ctx context.Context) (host.WholeOutput, error) {
	if j.link == nil {
		return host.WholeOutput{}, &host.Error{Kind: host.Offline, Message: "the job's runner is not connected"}
	}
	reader, err := filled(ctx, j.link, "JobRead", func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		return &runnerproto.JobRead{ID: id, JobID: j.id, Output: output}
	})
	if err != nil {
		return host.WholeOutput{}, err
	}
	data, err := collect(ctx, reader, runnerproto.JobKeptReadBytes)
	if err != nil {
		return host.WholeOutput{}, err
	}
	output, err := DecodeOutput(data, nil)
	if err != nil {
		return host.WholeOutput{}, &host.Error{
			Kind:    host.Protocol,
			Message: "the job's kept output does not decode: " + err.Error(),
		}
	}
	return output, nil
}

// Release tells the runner to remove the ended job's directory.
func (j *Job) Release(ctx context.Context) error {
	if j.link == nil {
		return nil
	}
	return j.link.send(ctx, &runnerproto.JobRelease{JobID: j.id})
}

// Kill signals the job; ended jobs do nothing.
func (j *Job) Kill(ctx context.Context, signal runnerproto.Signal) error {
	if !j.live() {
		return nil
	}
	return j.link.send(ctx, &runnerproto.JobKill{JobID: j.id, Signal: new(signal)})
}

// LogPage is a page of the Host's log with the next read's cursor.
type LogPage struct {
	// Lines contains the log entries returned by this read.
	Lines []runnerproto.LogLine
	// Next is the cursor for the next log read.
	Next uint64
}

// ServiceRequest describes a user stream or one-shot service invocation.
type ServiceRequest struct {
	// Context identifies the conversation and command locale.
	Context cmdproto.Context
	// Package is the descriptor of the invoked command package.
	Package cmdproto.PackageDescriptor
	// Operation names the package operation.
	Operation string
	// Args is the optional invocation argument object, encoded with contract codecs.
	Args json.RawMessage
	// JSON selects JSON output when supplied.
	JSON *bool
	// CWD is the invocation working directory.
	CWD string
	// Resolver locates executable artifacts for this service.
	Resolver ArtifactResolver
	// Attached lists additional artifacts to install for this invocation.
	Attached []AttachedArtifact
}

// AttachedArtifact allows a user stream to install an artifact beside its package's own.
type AttachedArtifact struct {
	// Artifact describes the artifact bytes.
	Artifact cmdproto.PackageArtifact
	// Location specifies where the runner downloads those bytes.
	Location cmdproto.ArtifactLocation
}

// ServiceEnd describes completion and the bounded stderr tail.
type ServiceEnd struct {
	// ExitCode is the service exit status.
	ExitCode uint8
	// Stderr contains the bounded diagnostic tail.
	Stderr string
}

// ServiceStream owns answers to a user stream's artifact requests until Close.
type ServiceStream struct {
	link    *Link
	id      string
	request ServiceRequest
	ctx     context.Context
	cancel  context.CancelFunc
	done    chan struct{}
	once    sync.Once
	result  ServiceEnd
	err     error
}

// Done waits for completion, failing if the runner leaves before reporting it.
func (s *ServiceStream) Done(ctx context.Context) (ServiceEnd, error) {
	select {
	case <-s.done:
		return s.result, s.err
	case <-ctx.Done():
		return ServiceEnd{}, ctx.Err()
	}
}

// Close cancels artifact requests and releases the stream. It is idempotent.
func (s *ServiceStream) Close() {
	s.link.mu.Lock()
	delete(s.link.services, s.id)
	s.link.mu.Unlock()
	s.cancel()
}

// ServiceExitError is a one-shot service call whose invocation exited nonzero.
type ServiceExitError struct {
	// ExitCode is the nonzero service exit status.
	ExitCode uint8
	// Stderr is the diagnostic tail of the failed service.
	Stderr string
	// Stdout is the collected output of the failed service.
	Stdout []byte
}

// Error describes the failed service call.
func (e *ServiceExitError) Error() string {
	return fmt.Sprintf("Service call exited with %d: %s", e.ExitCode, strings.TrimSpace(e.Stderr))
}

var _ host.Host = (*Host)(nil)

// connection resolves the Host's current device connection for each operation.
func (h *Host) connection() (*Link, error) {
	link := h.source().Link
	if link == nil {
		return nil, &host.Error{Kind: host.Offline, Message: "runner disconnected"}
	}
	if link.IsClosed() {
		return nil, link.offline()
	}
	return link, nil
}

// admit holds Cloud activity for precisely the operation's admitted lifetime.
func (h *Host) admit() (*gates.Lease, error) {
	if h.admission == nil {
		return nil, nil
	}
	return h.admission()
}

// mismatch reports an operation reply with the wrong shape.
func mismatch() error {
	return &host.Error{Kind: host.Protocol, Message: "the runner answered another operation"}
}

// finish publishes a service invocation's terminal result exactly once.
func (s *ServiceStream) finish(result ServiceEnd, err error) {
	s.once.Do(func() {
		s.result = result
		s.err = err
		s.cancel()
		close(s.done)
	})
}

// start sends a job's manifest and start contiguously with other job starts.
func (j *Job) start(ctx context.Context, job JobStart) (err error) {
	permit, err := j.link.jobTurn.Acquire(ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	defer func() {
		if err != nil {
			j.link.manifest = ""
		}
	}()
	var hash *string
	if job.Commands != nil {
		hash = new(job.Commands.Hash())
		if j.link.manifest != *hash {
			j.link.manifest = *hash
			if err := j.link.send(ctx, &runnerproto.ManifestMessage{Manifest: job.Commands.wire}); err != nil {
				return err
			}
		}
	}
	return j.link.send(
		ctx,
		&runnerproto.JobStart{
			JobID:        j.id,
			ManifestHash: hash,
			Context:      job.Context,
			Script:       job.Script,
			CWD:          job.CWD,
			Env:          job.Env,
			Stdin:        job.Stdin,
			Stdout:       job.Stdout,
		},
	)
}

// finish releases a running job's admission before publishing its terminal result.
func (j *Job) finish(end JobEnd) {
	j.finished.Do(func() {
		j.cancel()
		if j.lease != nil {
			j.lease.Release()
		}
		j.state.finish(end)
	})
}

// live reports whether a job still has a runner and no terminal result.
func (j *Job) live() bool {
	return j.link != nil && !j.state.hasEnded()
}

// sendStdin preserves stdin ordering while splitting writes to the runner's frame bound.
func sendStdin(ctx context.Context, link *Link, data []byte, frame func([]byte) runnerproto.Inbound) error {
	for len(data) > 0 {
		n := min(len(data), runnerproto.StdinChunkBytes)
		if err := link.send(ctx, frame(data[:n])); err != nil {
			return err
		}
		data = data[n:]
	}
	return nil
}

// collectService enforces the service call output bound and fails both pipes on overflow.
func collectService(ctx context.Context, reader *PipeReader, incoming, outgoing *Pipe, maxBytes int) ([]byte, error) {
	var output []byte
	for {
		chunk, readErr := reader.Next(ctx)
		if errors.Is(readErr, io.EOF) {
			break
		}
		if readErr != nil {
			return nil, &host.Error{Kind: host.Interrupted, Message: readErr.Error()}
		}
		if len(chunk) > maxBytes-len(output) {
			incoming.Fail("service call failed")
			outgoing.Fail("service call failed")
			//nolint:staticcheck // ST1005: product text, shown to the user as it is.
			return nil, fmt.Errorf("Service answer exceeds %d bytes", maxBytes)
		}
		output = append(output, chunk...)
	}
	return output, nil
}

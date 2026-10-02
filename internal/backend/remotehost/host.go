package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// DeviceLink is a device's current connection, or its last identity while offline.
// A nil Link means offline; Last is used only while offline.
type DeviceLink struct {
	Link *Link
	Last *host.Identity
}

// DeviceSourceFunc returns an atomic, immutable snapshot of a device's connection.
// The device owner synchronizes publication; a Host reloads it for each operation.
type DeviceSourceFunc func() DeviceLink

// Admission takes a Cloud admission lease without waiting, or refuses work.
// Nil means free admission, as on a paired device. The operation releases the lease.
type Admission func() (*gates.Lease, error)

// Host is a host.Host over a runner connection, including its job and service facets.
type Host struct{ _ byte }

// NewHost constructs a Host against the device owner's connection snapshots.
func NewHost(key host.Key, defaultCWD string, source DeviceSourceFunc, admission Admission) *Host {
	panic("not written: b-remotehost")
}

// Online reports whether the device's runner is connected.
func (h *Host) Online() bool { panic("not written: b-remotehost") }

// Key returns the execution target's value identity.
func (h *Host) Key() host.Key { panic("not written: b-remotehost") }

// DefaultCWD returns the base for relative paths.
func (h *Host) DefaultCWD() string { panic("not written: b-remotehost") }

// Identity returns the runner's current or last reported account.
func (h *Host) Identity() host.Identity { panic("not written: b-remotehost") }

// FS returns the Host's filesystem facet.
func (h *Host) FS() host.FS { panic("not written: b-remotehost") }

// Process returns the Host's process facet.
func (h *Host) Process() host.Process { panic("not written: b-remotehost") }

// StartJob starts one shell job. Offline jobs are already ended; admission may refuse them.
func (h *Host) StartJob(ctx context.Context, job JobStart) (*Job, error) {
	panic("not written: b-remotehost")
}

// ReadPipe opens the file before returning a pipe reader. Closing it stops the read.
func (h *Host) ReadPipe(ctx context.Context, path string, span host.ByteRange) (*PipeReader, error) {
	panic("not written: b-remotehost")
}

// WritePipe creates a pipe to the runner that this process fills.
func (h *Host) WritePipe() (*Pipe, error) { panic("not written: b-remotehost") }

// WriteFrom atomically replaces a file after input ends cleanly.
func (h *Host) WriteFrom(ctx context.Context, path string, input *Pipe, options host.WriteOptions) error {
	panic("not written: b-remotehost")
}

// GitChanges reads the working-tree changes beneath root.
func (h *Host) GitChanges(ctx context.Context, root string) (runnerwire.GitChanges, error) {
	panic("not written: b-remotehost")
}

// GitShow reads a file's committed contents from root.
func (h *Host) GitShow(ctx context.Context, root, path string) ([]byte, error) {
	panic("not written: b-remotehost")
}

// ReadLog reads up to limit lines after since, oldest first, optionally from one source.
func (h *Host) ReadLog(ctx context.Context, since *uint64, limit uint64, source *string) (LogPage, error) {
	panic("not written: b-remotehost")
}

// OpenNet connects a TCP stream between the socket and two pipes.
func (h *Host) OpenNet(ctx context.Context, hostname string, port uint16, input, output runnerwire.PipeRef) error {
	panic("not written: b-remotehost")
}

// OpenService opens a resident user stream. Streams are retention, not activity.
// The caller must Close the returned stream, including after Done.
func (h *Host) OpenService(ctx context.Context, request ServiceRequest, input, output runnerwire.PipeRef) (*ServiceStream, error) {
	panic("not written: b-remotehost")
}

// CallService sends one complete input and returns at most maxBytes of output.
// A nonzero exit reports its code, stderr tail and whole stdout in ServiceCallError.
func (h *Host) CallService(ctx context.Context, request ServiceRequest, input []byte, maxBytes int) ([]byte, error) {
	panic("not written: b-remotehost")
}

// ReleaseConversation retires the conversation's resident services.
func (h *Host) ReleaseConversation(ctx context.Context, conversation string) error {
	panic("not written: b-remotehost")
}

// HostIdentity converts the runner account to the Host's identity.
func HostIdentity(identity runnerwire.HostIdentity) host.Identity { panic("not written: b-remotehost") }

// JobStart describes one shell job and its optional declared commands and pipes.
type JobStart struct {
	Script string
	CWD    string
	// Env is exactly the variables above the device's environment.
	Env      map[string]string
	Context  commandwire.CommandContext
	Caller   *host.JobCaller
	Commands *CommandSelection
	Stdin    *runnerwire.PipeRef
	Stdout   *runnerwire.PipeRef
}

// Job is one shell job on the runner. The owner releases it after keeping its output and edits.
type Job struct{ _ byte }

// ID returns the runner's job identifier.
func (j *Job) ID() string { panic("not written: b-remotehost") }

// NextOutput returns a view, or io.EOF after the job ends and all views are taken.
func (j *Job) NextOutput(ctx context.Context) (JobOutput, error) { panic("not written: b-remotehost") }

// Follow starts or stops output beyond each stream's initial view; ended jobs do nothing.
func (j *Job) Follow(ctx context.Context, follow bool) error { panic("not written: b-remotehost") }

// RunningHint returns guidance from the latest declared command that supplies it.
func (j *Job) RunningHint() *string { panic("not written: b-remotehost") }

// Ended returns the terminal result, or nil while running.
func (j *Job) Ended() *JobEnd { panic("not written: b-remotehost") }

// End waits for the job's terminal result.
func (j *Job) End(ctx context.Context) (JobEnd, error) { panic("not written: b-remotehost") }

// WriteStdin writes ordered frames within the runner's stdin chunk limit.
func (j *Job) WriteStdin(ctx context.Context, bytes []byte) error { panic("not written: b-remotehost") }

// CloseStdin ends the job's standard input; ended jobs do nothing.
func (j *Job) CloseStdin(ctx context.Context) error { panic("not written: b-remotehost") }

// ReadOutput reads the kept output while running or ended, until release.
func (j *Job) ReadOutput(ctx context.Context) (host.WholeOutput, error) {
	panic("not written: b-remotehost")
}

// Release tells the runner to remove the ended job's directory.
func (j *Job) Release(ctx context.Context) error { panic("not written: b-remotehost") }

// Kill signals the job; ended jobs do nothing.
func (j *Job) Kill(ctx context.Context, signal runnerwire.Signal) error {
	panic("not written: b-remotehost")
}

// LogPage is a page of the Host's log with the next read's cursor.
type LogPage struct {
	Lines []runnerwire.LogLine
	Next  uint64
}

// ServiceRequest describes a user stream or one-shot service invocation.
type ServiceRequest struct {
	Context   commandwire.CommandContext
	Package   commandwire.PackageDescriptor
	Operation string
	// Args is the optional invocation argument object, encoded with contract codecs.
	Args     json.RawMessage
	JSON     *bool
	CWD      string
	Resolver ArtifactResolver
	Attached []AttachedArtifact
}

// AttachedArtifact allows a user stream to install an artifact beside its package's own.
type AttachedArtifact struct {
	Artifact commandwire.PackageArtifact
	Location commandwire.ArtifactLocation
}

// ServiceEnd describes completion and the bounded stderr tail.
type ServiceEnd struct {
	ExitCode uint8
	Stderr   string
}

// ServiceStream owns answers to a user stream's artifact requests until Close.
type ServiceStream struct{ _ byte }

// Done waits for completion, failing if the runner leaves before reporting it.
func (s *ServiceStream) Done(ctx context.Context) (ServiceEnd, error) {
	panic("not written: b-remotehost")
}

// Close cancels artifact requests and releases the stream. It is idempotent.
func (s *ServiceStream) Close() { panic("not written: b-remotehost") }

// ServiceCallErrorKind identifies a one-shot service failure.
type ServiceCallErrorKind uint8

const (
	// ServiceHostError wraps a Host operation failure.
	ServiceHostError ServiceCallErrorKind = iota
	// ServiceExited means the invocation exited nonzero.
	ServiceExited
	// ServiceTooLarge means stdout exceeded the requested bound.
	ServiceTooLarge
)

// ServiceCallError carries a Host failure, nonzero completion, or output limit.
type ServiceCallError struct {
	Kind     ServiceCallErrorKind
	Err      error
	ExitCode uint8
	Stderr   string
	Stdout   []byte
	Limit    int
}

// Error describes the failed service call.
func (e *ServiceCallError) Error() string { panic("not written: b-remotehost") }

// Unwrap exposes an underlying Host failure for errors.Is and errors.As.
func (e *ServiceCallError) Unwrap() error { panic("not written: b-remotehost") }

var _ host.Host = (*Host)(nil)

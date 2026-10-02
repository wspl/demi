package jobs

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// ExecutionContext is the live authority a job's declared commands run under.
// Published fields and the manifest are immutable. Its creator must Close it
// after the job and all its command invocations have ended.
type ExecutionContext struct {
	ID    string
	JobID string
	// Command is what the backend told the job's declared commands.
	Command  commandwire.CommandContext
	Manifest *runnerwire.Manifest
	// Edits is where the job records the files its commands change.
	Edits commandwire.EditContext
	// Connection carries callbacks and artifact locations for this job.
	Connection *ConnectionHandle
	lifetime   context.Context
	cancel     context.CancelFunc
	aliases    string
	closeOnce  sync.Once
	closeErr   error
}

// NewExecutionContext creates private command aliases. The context becomes live
// only when registered on its connection. ctx owns cancellation of its work.
func NewExecutionContext(ctx context.Context, jobID string, command commandwire.CommandContext, manifest *runnerwire.Manifest, edits commandwire.EditContext, connection *ConnectionHandle, paths ContextPaths) (*ExecutionContext, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	aliases, err := os.MkdirTemp(paths.Directory, "client-")
	if err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			_ = os.RemoveAll(aliases)
		}
	}() // Failed construction owns no aliases.
	for name := range manifest.Roots {
		destination := filepath.Join(aliases, name)
		if runtime.GOOS == "windows" {
			destination += ".exe"
			err = os.Link(paths.Executable, destination)
			if errors.Is(err, syscall.Errno(17)) {
				var bytes []byte
				bytes, err = os.ReadFile(paths.Executable)
				if err == nil {
					err = os.WriteFile(destination, bytes, 0700)
				}
			}
		} else {
			err = os.Symlink(paths.Executable, destination)
		}
		if err != nil {
			return nil, err
		}
	}
	lifetime, cancel := context.WithCancel(ctx)
	success = true
	return &ExecutionContext{ID: executionID(), JobID: jobID, Command: command, Manifest: manifest, Edits: edits, Connection: connection, lifetime: lifetime, cancel: cancel, aliases: aliases}, nil
}

// Environment supplies the local endpoint, context ID, DEMI_HOME and alias-first
// PATH. A nil path means PATH was absent, rather than present and empty.
func (e *ExecutionContext) Environment(endpoint, home string, path *string) (map[string]string, error) {
	aliasPath := e.aliases
	if runtime.GOOS == "windows" {
		if strings.ContainsRune(aliasPath, '"') {
			return nil, errors.New("path segment contains `\"`")
		}
		if strings.ContainsRune(aliasPath, ';') {
			aliasPath = `"` + aliasPath + `"`
		}
	} else if strings.ContainsRune(aliasPath, ':') {
		return nil, errors.New("path segment contains separator `:`")
	}
	// Preserve caller PATH entries, including a present empty entry and Windows quoting.
	if path != nil {
		aliasPath += string(os.PathListSeparator) + *path
	}
	return map[string]string{process.EndpointEnv: endpoint, process.ContextEnv: e.ID, "DEMI_HOME": home, "PATH": aliasPath}, nil
}

// Carries reports whether this context's manifest carries digest for this Host.
func (e *ExecutionContext) Carries(digest string) bool {
	target, err := commandwire.HostTarget()
	if err != nil {
		return false
	}
	for _, pkg := range e.Manifest.Packages {
		if _, ok := pkg.Carries(target, digest); ok {
			return true
		}
	}
	return false
}

// Done closes when the execution authority is cancelled.
func (e *ExecutionContext) Done() <-chan struct{} {
	return e.lifetime.Done()
}

// Cancel revokes running invocations without removing the context's aliases.
func (e *ExecutionContext) Cancel() {
	e.cancel()
}

// Close cancels the context and removes its aliases. Its owner first joins all
// invocations. Repeated calls are harmless.
func (e *ExecutionContext) Close(_ context.Context) error {
	e.Cancel()
	e.closeOnce.Do(func() { e.closeErr = os.RemoveAll(e.aliases) })
	return e.closeErr
}

// ContextPaths names the private context directory and the executable aliases run.
type ContextPaths struct {
	Directory  string
	Executable string
}

// NewContextPaths creates and protects the directory for manifests and aliases.
func NewContextPaths(ctx context.Context, directory, executable string) (ContextPaths, error) {
	if err := ctx.Err(); err != nil {
		return ContextPaths{}, err
	}
	if err := os.MkdirAll(directory, 0700); err != nil {
		return ContextPaths{}, err
	}
	if err := process.Chmod(ctx, directory, 0700); err != nil {
		return ContextPaths{}, err
	}
	return ContextPaths{Directory: directory, Executable: executable}, nil
}

// Contexts provides concurrent lookup in the connection's immutable snapshot.
// Its zero value contains no contexts and may be shared across reconnects.
type Contexts struct{ snapshot atomic.Pointer[contextIndex] }
type contextIndex map[string]*ExecutionContext

// Lookup returns the live context with id or a permission error.
func (c *Contexts) Lookup(id string) (*ExecutionContext, error) {
	if snapshot := c.snapshot.Load(); snapshot != nil {
		if found := (*snapshot)[id]; found != nil {
			return found, nil
		}
	}
	return nil, contextUnavailable{}
}

// Carrying finds an uncancelled context authorizing digest for this Host.
func (c *Contexts) Carrying(digest string) (*ExecutionContext, bool) {
	if snapshot := c.snapshot.Load(); snapshot != nil {
		for _, entry := range *snapshot {
			if entry.lifetime.Err() == nil && entry.Carries(digest) {
				return entry, true
			}
		}
	}
	return nil, false
}

// ContextTable owns a connection's registrations and service leases. The
// connection serializes its calls and closes it before publishing a new table.
// ExecutionContext creators retain responsibility for removing their aliases.
type ContextTable struct {
	contexts *Contexts
	entries  contextIndex
	leases   map[string][]*cmdpkgs.ServiceLease
}

// NewContextTable clears and publishes registrations through contexts.
func NewContextTable(contexts *Contexts) *ContextTable {
	t := &ContextTable{contexts: contexts, entries: make(contextIndex), leases: make(map[string][]*cmdpkgs.ServiceLease)}
	t.publish()
	return t
}

// Insert makes execution live; a job has at most one context. Success transfers
// leases to the table; on error the caller remains responsible for releasing them.
func (t *ContextTable) Insert(execution *ExecutionContext, leases []*cmdpkgs.ServiceLease) error {
	for _, existing := range t.entries {
		if existing.JobID == execution.JobID {
			return errors.New("duplicate live execution owner")
		}
	}
	t.entries[execution.ID] = execution
	t.leases[execution.ID] = leases
	t.publish()
	return nil
}

// Cancel cancels jobID's context, retaining its registration until the job ends.
func (t *ContextTable) Cancel(jobID string) {
	for _, entry := range t.entries {
		if entry.JobID == jobID {
			entry.Cancel()
		}
	}
}

// Remove revokes jobID's registration and releases its leases after its IO ends.
func (t *ContextTable) Remove(jobID string) {
	for id, entry := range t.entries {
		if entry.JobID == jobID {
			entry.Cancel()
			delete(t.entries, id)
			leases := t.leases[id]
			delete(t.leases, id)
			t.publish()
			for _, lease := range leases {
				lease.Release()
			}
			return
		}
	}
}

// Close revokes every registration and releases every lease. It is idempotent.
func (t *ContextTable) Close() {
	for _, entry := range t.entries {
		t.Remove(entry.JobID)
	}
}

// InstallationPhase describes the connection's manifest selection.
type InstallationPhase uint8

const (
	// ManifestAbsent means no manifest is installed.
	ManifestAbsent InstallationPhase = iota
	// ManifestInstalling means the latest manifest is being checked and kept.
	ManifestInstalling
	// ManifestReady means a checked manifest is available.
	ManifestReady
)

// Installation publishes the selected manifest to jobs. Its zero value is absent.
// Methods are safe for concurrent use; manifest values are immutable after publication.
type Installation struct {
	// mu protects the selection and its notification channel, never IO.
	mu       sync.Mutex
	phase    InstallationPhase
	manifest *runnerwire.Manifest
	changed  chan struct{}
}

// Publish replaces the selection and wakes waiting jobs. manifest is non-nil
// exactly when phase is ManifestReady. The connection serializes publications.
func (i *Installation) Publish(phase InstallationPhase, manifest *runnerwire.Manifest) {
	i.mu.Lock()
	old := i.changed
	i.changed = make(chan struct{})
	i.phase = phase
	i.manifest = manifest
	i.mu.Unlock()
	if old != nil {
		close(old)
	}
}

// Wait waits until installation is no longer in progress and returns its selection.
// The manifest is nil for ManifestAbsent. ctx ends with the job or connection.
func (i *Installation) Wait(ctx context.Context) (InstallationPhase, *runnerwire.Manifest, error) {
	for {
		i.mu.Lock()
		phase, manifest := i.phase, i.manifest
		if i.changed == nil {
			i.changed = make(chan struct{})
		}
		changed := i.changed
		i.mu.Unlock()
		if phase != ManifestInstalling {
			return phase, manifest, nil
		}
		select {
		case <-ctx.Done():
			return ManifestAbsent, nil, ctx.Err()
		case <-changed:
		}
	}
}

// Installed holds a checked manifest and the service leases keeping it resident.
// The connection releases Leases when replacing this installed selection.
type Installed struct {
	Manifest *runnerwire.Manifest
	Leases   []*cmdpkgs.ServiceLease
}

// Install decodes, checks and keeps the manifest, refusing reserved root commands.
// It acquires leases for this Host's packages; failure releases partial acquisitions.
func Install(ctx context.Context, value json.RawMessage, paths ContextPaths, services *cmdpkgs.ServiceHandle, reserved map[string]struct{}) (*Installed, error) {
	manifest, err := runnerwire.DecodeManifest(value)
	if err != nil {
		return nil, err
	}
	for name := range manifest.Roots {
		if _, found := reserved[name]; found {
			return nil, fmt.Errorf("reserved root command: %s", name)
		}
	}
	path := filepath.Join(paths.Directory, manifest.Hash+".json")
	bytes, err := os.ReadFile(path)
	switch {
	case err == nil:
		existing, err := runnerwire.DecodeManifest(bytes)
		if err != nil {
			return nil, err
		}
		if existing.Hash != manifest.Hash {
			return nil, errors.New("cached manifest identity mismatch")
		}
	case errors.Is(err, os.ErrNotExist):
		bytes, err = manifest.MarshalJSON()
		if err != nil {
			return nil, err
		}
		if err = process.WritePrivate(ctx, path, bytes); err != nil {
			return nil, err
		}
	default:
		return nil, err
	}
	leases, err := Leases(ctx, &manifest, services)
	if err != nil {
		return nil, err
	}
	return &Installed{Manifest: &manifest, Leases: leases}, nil
}

// Leases acquires the manifest's services for this Host. The caller releases all
// returned leases; failure releases acquisitions already made.
func Leases(ctx context.Context, manifest *runnerwire.Manifest, services *cmdpkgs.ServiceHandle) ([]*cmdpkgs.ServiceLease, error) {
	var leases []*cmdpkgs.ServiceLease
	target, err := commandwire.HostTarget()
	if err != nil {
		return nil, err
	}
	for _, pkg := range manifest.Packages {
		if artifact, ok := pkg.Targets[string(target)]; ok {
			lease, err := services.Lease(ctx, artifact.SHA256)
			if err != nil {
				for _, held := range leases {
					held.Release()
				}
				return nil, err
			}
			leases = append(leases, lease)
		}
	}
	return leases, nil
}

// publish replaces the connection's immutable execution authority snapshot.
func (t *ContextTable) publish() {
	snapshot := maps.Clone(t.entries)
	t.contexts.snapshot.Store(&snapshot)
}

// executionID names one runner-owned job context or callback without exposing authority data.
func executionID() string {
	var bytes [16]byte
	_, _ = rand.Read(bytes[:])
	return hex.EncodeToString(bytes[:])
}

// contextUnavailable retains the protocol diagnostic and permission classification.
type contextUnavailable struct{}

func (contextUnavailable) Error() string { return "execution context is not live on this runner" }
func (contextUnavailable) Unwrap() error { return os.ErrPermission }

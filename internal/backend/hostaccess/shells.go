package hostaccess

import (
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// ConversationBlobs adapts the owner's namespace and use records for storage.
type ConversationBlobs struct{ Namespace *blobs.Namespace }

// Media is the namespace as the conversation's sessions reach it.
func (b ConversationBlobs) Media() store.BlobStore {
	return b.Namespace
}

// CommitUses records references inside a database commit without network IO.
func (b ConversationBlobs) CommitUses(_ context.Context, refs []core.BlobRef) error {
	return b.Namespace.CommitUses(refs)
}

// ConversationHostForNode resolves the current main Host for an agent node.
// The returned handle is for shell environment identity; its jobs must use
// RunJob, and other file operations must use WithHost for their whole lifetime.
func ConversationHostForNode(ctx context.Context, shard HostShard, id webapi.ConversationID) (*remotehost.Host, error) {
	identity, err := WithHost(
		ctx,
		shard,
		id,
		nil,
		func(_ context.Context, admitted *ConversationHost) (*remotehost.Host, error) {
			account := admitted.Host.Identity()
			return remotehost.NewHost(
				admitted.Host.Key(),
				admitted.Root,
				func() remotehost.DeviceLink { return remotehost.DeviceLink{Last: &account} },
				func() (*gates.Lease, error) {
					return nil, &host.Error{
						Kind:    host.Unavailable,
						Message: "the conversation's Host changed; the command did not run",
					}
				},
			), nil
		},
	)
	return identity, asHostError(err)
}

// RunJob admits one shell job, checks its Host identity, installs the user's
// plugin directories once per connection and revision, then runs it once.
// JobEnded runs on every completion, failure or cancellation after dispatch.
func RunJob(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	key host.Key,
	job func(context.Context) error,
) error {
	return runJob(ctx, shard, id, key, func(ctx context.Context, _ *Admitted) error { return job(ctx) })
}

// ShardShellEnvironments makes node-owned shell environments and keeps outputs
// and edit copies as the user's blobs. The shard closes environments before its
// dependencies; the factory owns no goroutines independently of those scopes.
type ShardShellEnvironments struct {
	shard   HostShard
	catalog *remotehost.CommandCatalog
}

// NewShardShellEnvironments binds a shard and its published command catalog.
func NewShardShellEnvironments(shard HostShard, catalog *remotehost.CommandCatalog) *ShardShellEnvironments {
	return &ShardShellEnvironments{shard: shard, catalog: catalog}
}

// Create registers the node's commands and makes its environment on target.
// The receiver owns and closes the returned environment.
func (s *ShardShellEnvironments) Create(
	ctx context.Context,
	scope tools.EnvironmentScope,
	target *remotehost.Host,
) (host.ShellEnvironment, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	id := ConversationOf(scope.Root)
	deviceText, ok := runners.DeviceOf(target.Key())
	if !ok {
		return nil, &host.Error{Kind: host.Protocol, Message: "the job's Host is no conversation's"}
	}
	device, err := webapi.ParseDeviceID(deviceText)
	if err != nil {
		return nil, &host.Error{Kind: host.Protocol, Message: "the job's Host is no conversation's"}
	}
	selection, err := s.catalog.Select(scope.Commands)
	if err != nil {
		return nil, &host.Error{Kind: host.Protocol, Message: err.Error()}
	}
	bridge := &shellAdmission{shard: s.shard, id: id, active: make(map[*Admitted]struct{})}
	private := remotehost.NewHost(target.Key(), target.DefaultCWD(), func() remotehost.DeviceLink {
		return remotehost.DeviceLink{Link: s.shard.Devices().Link(device)}
	}, bridge.admit)
	source := func(ctx context.Context) (commandwire.CommandContext, error) {
		return runners.CommandContext(
			ctx,
			s.shard.Control(),
			s.shard.User(),
			id,
			&commandwire.AgentCaller{Number: scope.Agent},
		)
	}
	options := remotehost.NewEnvironmentOptions(private, source, scope.Feed, scope.Numbers)
	options.Commands = selection
	options.Access = bridge
	options.Keeper = &commandKeeper{shard: s.shard, id: id, host: private}
	environment := remotehost.NewShellEnvironment(options)
	registration := s.shard.Commands().Register(string(scope.Node), id, scope.Commands, selection)
	return &registeredEnvironment{ShellEnvironment: environment, registration: registration}, nil
}

var (
	_ database.OwnerBlobs                             = ConversationBlobs{}
	_ tools.ShellEnvironmentFactory[*remotehost.Host] = (*ShardShellEnvironments)(nil)
)

// runJob keeps the checked Host admission through job IO and output storage.
func runJob(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	key host.Key,
	job func(context.Context, *Admitted) error,
) error {
	text, ok := runners.DeviceOf(key)
	if !ok {
		return &host.Error{Kind: host.Protocol, Message: "the job's Host is no conversation's"}
	}
	device, err := webapi.ParseDeviceID(text)
	if err != nil {
		return &host.Error{Kind: host.Protocol, Message: "the job's Host is no conversation's"}
	}
	admitted, err := AdmitHost(ctx, shard, id, &device)
	if err != nil {
		return asHostError(err)
	}
	defer admitted.Release()
	if admitted.Host.Host.Key() != key {
		return &host.Error{Kind: host.Unavailable, Message: "the conversation's Host changed; the command did not run"}
	}
	if err := installDirectories(ctx, shard, device, &admitted.Host); err != nil {
		return err
	}
	defer shard.JobEnded(id)
	return job(ctx, admitted)
}

// asHostError translates product admission failures for the agent tools.
func asHostError(err error) error {
	if err == nil {
		return nil
	}
	var failure *host.Error
	if errors.As(err, &failure) {
		return failure
	}
	kind := host.Unavailable
	var access *Error
	if errors.As(err, &access) {
		switch access.Kind {
		case AccessCancelled:
			kind = host.Interrupted
		case AccessStorage:
			kind = host.Failed
		}
	}
	return &hostAdmissionError{failure: &host.Error{Kind: kind, Message: err.Error()}, cause: err}
}

// shellAdmission is private to an environment: its operational Host never
// escapes to the node, and only jobs holding an admission can use it.
type shellAdmission struct {
	shard  HostShard
	id     webapi.ConversationID
	mu     sync.Mutex // Protects active job admissions, never IO or callbacks.
	active map[*Admitted]struct{}
}

// RunJob keeps an active admission available to this environment’s Host operations.
func (s *shellAdmission) RunJob(ctx context.Context, key host.Key, job func(context.Context) error) error {
	return runJob(ctx, s.shard, s.id, key, func(ctx context.Context, admitted *Admitted) error {
		s.mu.Lock()
		s.active[admitted] = struct{}{}
		s.mu.Unlock()
		defer func() {
			s.mu.Lock()
			delete(s.active, admitted)
			s.mu.Unlock()
		}()
		return job(ctx)
	})
}

// admit uses any active job's hold: all belong to this environment's same
// checked Host, and no reset can replace its Cloud while those holds remain.
func (s *shellAdmission) admit() (*gates.Lease, error) {
	s.mu.Lock()
	var admission *Admitted
	for active := range s.active {
		admission = active
		break
	}
	s.mu.Unlock()
	if admission == nil {
		return nil, &host.Error{
			Kind:    host.Unavailable,
			Message: "the conversation's Host changed; the command did not run",
		}
	}
	// The calling job retains its own admission throughout this private operation.
	// A sibling may release the sampled hold, but all live jobs share this Cloud.
	if admission.perOperation != nil {
		return admission.perOperation()
	}
	return nil, nil
}

type registeredEnvironment struct {
	host.ShellEnvironment
	registration *runners.CommandRegistration
	once         sync.Once
}

// DisposeAll ends the environment before releasing its command registration.
func (e *registeredEnvironment) DisposeAll(ctx context.Context) error {
	err := e.ShellEnvironment.DisposeAll(ctx)
	e.once.Do(e.registration.Release)
	return err
}

type commandKeeper struct {
	shard HostShard
	id    webapi.ConversationID
	host  *remotehost.Host
}

// Retain keeps each readable edit pair without losing records for missing copies.
func (k *commandKeeper) Retain(
	ctx context.Context,
	command core.CommandID,
	files []runnerwire.JobFileChange,
) ([]core.EditedFile, error) {
	retained := make([]core.EditedFile, 0, len(files))
	for _, file := range files {
		copies := make([]*core.EditCopies, len(file.Edits))
		for index, segment := range file.Edits {
			if segment.Modified == nil {
				continue
			}
			stored, err := k.storeCopies(ctx, segment.Original, *segment.Modified)
			if err != nil {
				slog.Warn(
					"an edit's copies were not stored",
					"conversation",
					k.id,
					"command",
					command,
					"path",
					file.Path,
					"error",
					err,
				)
			} else {
				copies[index] = stored
			}
		}
		retained = append(
			retained,
			remotehost.EditedFile(file, func(index int) *core.EditCopies { return copies[index] }),
		)
	}
	return retained, nil
}

// storeCopies stores before and after as text, including an empty original for creation.
func (k *commandKeeper) storeCopies(ctx context.Context, original *string, modified string) (*core.EditCopies, error) {
	before := ""
	if original != nil {
		data, err := k.host.FS().ReadFile(ctx, *original)
		if err != nil {
			return nil, err
		}
		before, err = runners.TextOf(data)
		if err != nil {
			return nil, err
		}
	}
	data, err := k.host.FS().ReadFile(ctx, modified)
	if err != nil {
		return nil, err
	}
	after, err := runners.TextOf(data)
	if err != nil {
		return nil, err
	}
	first, err := k.shard.Blobs().Put(ctx, []byte(before))
	if err != nil {
		return nil, err
	}
	second, err := k.shard.Blobs().Put(ctx, []byte(after))
	if err != nil {
		return nil, err
	}
	return &core.EditCopies{Original: first, Modified: second}, nil
}

// KeepOutput records either the kept blob or why it was not stored; storage
// failures are logged and never prevent a command from ending.
func (k *commandKeeper) KeepOutput(ctx context.Context, command core.CommandID, output host.WholeOutput) error {
	ended := k.shard.Clock().Now()
	encoded, err := remotehost.EncodeOutput(output)
	var blob core.BlobRef
	if err == nil {
		blob, err = k.shard.Blobs().Put(ctx, encoded)
	}
	var stored database.OutputRow
	if err != nil {
		slog.Warn("a command's output was not stored", "conversation", k.id, "command", command, "error", err)
		stored = &database.OutputNotStored{Reason: err.Error()}
	} else {
		stored = &database.OutputStored{Blob: blob, Missing: output.Missing}
	}
	row := database.CommandOutput{Command: command, Ended: ended, Output: stored}
	err = k.shard.ConversationDB(k.id).Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		return database.InsertCommandOutputs(
			ctx,
			tx,
			ConversationBlobs{Namespace: k.shard.Blobs()},
			[]database.CommandOutput{row},
		)
	})
	if err != nil {
		slog.Warn("a command's output was not recorded", "conversation", k.id, "command", command, "error", err)
	}
	return nil
}

// hostAdmissionError preserves both the tool-facing classification and the cause.
type hostAdmissionError struct {
	failure *host.Error
	cause   error
}

// Error returns the tool-facing admission message.
func (e *hostAdmissionError) Error() string { return e.failure.Error() }

// Unwrap preserves both the classification and underlying admission failure.
func (e *hostAdmissionError) Unwrap() []error { return []error{e.failure, e.cause} }

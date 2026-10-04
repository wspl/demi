package usershard

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// User identifies the owner.
func (s *Shard) User() webapiproto.UserID {
	return s.user
}

// Cloud is the user's Cloud machine.
func (s *Shard) Cloud() *cloud.Cloud {
	return s.cloud
}

// Devices is the user's devices, the Cloud among them.
func (s *Shard) Devices() *runners.Devices {
	return &s.devices
}

// Control supplies the user's durable control records.
func (s *Shard) Control() *database.ControlService {
	return s.services.Control
}

// CloudServices supplies the shared manager client, capacity and settings.
func (s *Shard) CloudServices() *cloud.Services {
	return s.services.Cloud
}

// PublicURL is where a booting Cloud's runner reaches this backend.
func (s *Shard) PublicURL() *runners.PublicURL {
	return s.services.PublicURL
}

// IdleWindow is how long a Cloud no conversation uses stays awake.
func (s *Shard) IdleWindow() time.Duration {
	return s.services.Lifecycle.IdleWindow
}

// Marks is the user's pages, which show the Cloud and the devices.
func (s *Shard) Marks() pagesync.UserMarks {
	return s.services.Sync.Of(s.user)
}

// Vault is the credential vault, whose entries say which providers run a
// process on the Cloud.
func (s *Shard) Vault() *providerhost.Vault {
	return s.services.Vault
}

// Assembly resolves which providers need a process.
func (s *Shard) Assembly() *providerhost.Assembly {
	return s.services.Assembly
}

// Activity is what the conversation is doing: a turn of its tree, an
// operation holding its file gate, or a user stream someone has open.
func (s *Shard) Activity(conversation webapiproto.ConversationID) idlewatch.Activity {
	slot := s.conversations.Slot(conversation)
	fileState := slot.FileGate().State()
	fileActivity := idlewatch.Of(fileState)
	streamState := slot.Streams().State()
	activity := fileActivity.And(idlewatch.Of(streamState))
	if tree := s.agent.Tree(hostaccess.RootOf(conversation)); tree != nil {
		treeState := tree.Admission().State()
		activity = activity.And(idlewatch.Of(treeState))
	}
	return activity
}

// Attended reports whether someone attends the conversation: a turn of it
// is in flight, or a file transfer or user stream of it is open.
func (s *Shard) Attended(conversation webapiproto.ConversationID) bool {
	if tree := s.agent.Tree(hostaccess.RootOf(conversation)); tree != nil && tree.Admission().State().Demand > 0 {
		return true
	}
	return s.conversations.Slot(conversation).Transfers().AnyOpen()
}

// HoldForIdle holds the conversation's tree and file gate now if neither
// works. Nil means either is held; a failed attempt releases partial holds.
func (s *Shard) HoldForIdle(conversation webapiproto.ConversationID) cloud.ConversationHold {
	hold := &conversationHold{}
	if tree := s.agent.Tree(hostaccess.RootOf(conversation)); tree != nil {
		hold.tree = tree.Admission().TryReserve()
		if hold.tree == nil {
			return nil
		}
	}
	hold.files = s.conversations.Slot(conversation).FileGate().TryReserve()
	if hold.files == nil {
		hold.Release()
		return nil
	}
	return hold
}

// HoldForReset interrupts the turn and holds its tree. If filesOnCloud,
// it ends file transfers and user streams, then reserves the file gate
// once its operations end. Each wait has hold; if one runs out, it returns
// ErrNotLetGo. Cancellation returns ctx.Err().
// Failure releases partial holds; success transfers Release to the caller.
func (s *Shard) HoldForReset(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	filesOnCloud bool,
	hold time.Duration,
) (cloud.ConversationHold, error) {
	return s.holdReset(ctx, conversation, filesOnCloud, hold)
}

// CloudStopped ends the exposes of a Cloud that stops or stopped.
func (s *Shard) CloudStopped(ctx context.Context, device webapiproto.DeviceID) error {
	return s.stopExposes(context.WithoutCancel(ctx), device)
}

// Clock is the wall clock the backend reads times from.
func (s *Shard) Clock() types.Clock {
	return s.services.Clock
}

// Pipes is the pipes of the user's devices.
func (s *Shard) Pipes() *remotehost.Pipes {
	return s.pipes
}

// Commands is each agent node's commands, for the rpc calls of its jobs.
func (s *Shard) Commands() *runners.CommandRouter {
	return &s.commands
}

// Conversations owns each conversation's slot, file gate and transfers.
func (s *Shard) Conversations() *hostaccess.Conversations {
	return s.conversations
}

// Blobs is the user's blob namespace.
func (s *Shard) Blobs() *blobs.Namespace {
	return s.services.Blobs.ForUser(s.user)
}

// ConversationDB is the database of the user's conversation.
func (s *Shard) ConversationDB(conversation webapiproto.ConversationID) *database.ConversationDB {
	return s.services.Conversations.DB(conversation)
}

// Native is the command packages the conversations' commands bind to.
func (s *Shard) Native() *runners.NativeCatalog {
	return s.services.Native
}

// CloudShard is the shard as the user's Cloud sees it.
func (s *Shard) CloudShard() cloud.Shard {
	return s
}

// TrackIdle starts the conversation's idle watch unless one runs.
func (s *Shard) TrackIdle(conversation webapiproto.ConversationID) {
	s.mu.Lock()
	if s.closing || s.idle[conversation] != nil {
		s.mu.Unlock()
		return
	}
	ctx, cancel := context.WithCancel(s.ctx)
	watch := &idleWatch{cancel: cancel, done: make(chan struct{})}
	if s.idle == nil {
		s.idle = make(map[webapiproto.ConversationID]*idleWatch)
	}
	s.idle[conversation] = watch
	s.work.Add(1)
	s.mu.Unlock()
	go func() {
		defer s.work.Done()
		defer close(watch.done)
		defer cancel()
		idlewatch.Watch(
			ctx,
			conversationIdle{shard: s, id: conversation},
			s.services.Lifecycle.IdleWindow,
			s.services.Lifecycle.IdlePoll,
		)
		s.mu.Lock()
		if s.idle[conversation] == watch {
			delete(s.idle, conversation)
		}
		s.mu.Unlock()
	}()
}

// DirectorySets is the Host directories of the user's plugins.
func (s *Shard) DirectorySets(ctx context.Context) (hostaccess.DirectorySets, error) {
	sets, err := s.plugins.Directories(ctx)
	if err != nil {
		return nil, fmt.Errorf("the plugins' Host directories cannot be read: %w", err)
	}
	result := make(hostaccess.DirectorySets, len(sets))
	for i, set := range sets {
		result[i] = hostaccess.DirectorySet{Plugin: set.Plugin, Directories: set.Directories}
	}
	return result, nil
}

// PluginInstalls remembers the Hosts' installed directories.
func (s *Shard) PluginInstalls() *hostaccess.PluginInstalls {
	return s.installs
}

// JobEnded reports a finished or stopped job, which may change plugin views.
func (s *Shard) JobEnded(conversation webapiproto.ConversationID) {
	s.mu.Lock()
	s.jobsEnded[conversation]++
	s.mu.Unlock()
	s.plugins.Fire(plugin.TopicJobs, &conversation)
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: conversation})
}

// PackageCall runs operation on the conversation's main Host, waking it as kind
// specifies. Args is a JSON object preserving its input member order.
func (s *Shard) PackageCall(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	operation commanddecl.NativeOperation,
	args json.RawMessage,
	kind plugin.CallKind,
) (json.RawMessage, error) {
	packageDefinition, ok := s.services.Native.Package(operation.Package)
	if !ok {
		return nil, fmt.Errorf("the catalog serves no such package")
	}
	callKind := hostaccess.Starts
	switch kind {
	case plugin.CallKindStarts:
		callKind = hostaccess.Starts
	case plugin.CallKindOperates:
		callKind = hostaccess.Operates
	case plugin.CallKindLooks:
		callKind = hostaccess.Looks
	}
	answer, err := hostaccess.UserCall(
		ctx,
		s,
		conversation,
		callKind,
		hostaccess.ServiceCall{
			Binding:  hostaccess.ServiceBinding{Package: packageDefinition, Operation: operation.Operation},
			Args:     args,
			MaxBytes: 1024 * 1024,
		},
	)
	if err != nil {
		return nil, callFailure(err)
	}
	if err := contract.CheckJSON(answer); err != nil {
		return nil, fmt.Errorf("the operation answered what is not JSON: %w", err)
	}
	return answer, nil
}

// ConversationHosts lists the conversation's main and attached Hosts.
func (s *Shard) ConversationHosts(
	ctx context.Context,
	conversation webapiproto.ConversationID,
) ([]plugin.ConversationHost, error) {
	hosts, err := hostaccess.ConversationHosts(ctx, s, conversation)
	if err != nil {
		return nil, accessFailure(err)
	}
	result := make([]plugin.ConversationHost, 0, len(hosts))
	for _, h := range hosts {
		role := plugin.HostRoleMain
		if h.Role == hostaccess.Attached {
			role = plugin.HostRoleAttached
		}
		result = append(
			result,
			plugin.ConversationHost{
				Name:   h.Name,
				Device: h.Device,
				Role:   role,
				Online: s.devices.Online(h.Device),
			},
		)
	}
	return result, nil
}

// ReadHostFiles never wakes a Host; a stopped Host returns plugin.PortRefusalNotRunning.
func (s *Shard) ReadHostFiles(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	reads []plugin.HostRead,
) ([]plugin.HostFile, error) {
	files, err := hostaccess.ReadFiles(ctx, s, conversation, reads)
	if errors.Is(err, hostaccess.ErrNotRunning) {
		return nil, &plugin.PortRefusalNotRunning{}
	}
	return files, accessFailure(err)
}

// PutBlob stores bytes in the user's blob namespace.
func (s *Shard) PutBlob(ctx context.Context, bytes types.B64Bytes) (types.BlobRef, error) {
	return s.Blobs().Put(ctx, bytes)
}

// Blob returns the user's blob bytes, or nil when absent.
func (s *Shard) Blob(ctx context.Context, blob types.BlobRef) (*types.B64Bytes, error) {
	bytes, ok, err := s.Blobs().Read(ctx, blob)
	if err != nil || !ok {
		return nil, err
	}
	return &bytes, nil
}

// BlobUses returns the record updated before a plugin value or directory write commits.
func (s *Shard) BlobUses() database.OwnerBlobs {
	return hostaccess.ConversationBlobs{Namespace: s.Blobs()}
}

// Exposes lists the user's live exposes, soonest expiry first.
func (s *Shard) Exposes(ctx context.Context) (plugin.ExposeList, error) {
	values, err := expose.List(ctx, s.ExposeShard())
	if err != nil {
		return plugin.ExposeList{}, err
	}
	records := make([]plugin.ExposeRecord, 0, len(values))
	for _, value := range values {
		record, err := s.portExpose(ctx, value)
		if err != nil {
			return plugin.ExposeList{}, err
		}
		records = append(records, record)
	}
	return plugin.ExposeList{
		Available: s.services.ExposeDomain != nil,
		ListedAt:  s.Clock().Now(),
		Exposes:   records,
	}, nil
}

// CreateExpose exposes address on device for lifetime seconds.
func (s *Shard) CreateExpose(
	ctx context.Context,
	device webapiproto.DeviceID,
	address string,
	lifetime uint64,
) (plugin.ExposeRecord, error) {
	parsed, err := webapiproto.ParseExposeAddress(address)
	if err != nil {
		return plugin.ExposeRecord{}, &plugin.PortRefusalExpose{
			Reason:  plugin.ExposeRefusalInvalidAddress,
			Message: "the address " + err.Error(),
		}
	}
	duration, err := exposeLifetime(lifetime)
	if err != nil {
		return plugin.ExposeRecord{}, err
	}
	value, err := expose.Add(ctx, s.ExposeShard(), device, parsed, duration)
	if err != nil {
		return plugin.ExposeRecord{}, exposeFailure(err)
	}
	return s.portExpose(ctx, value)
}

// RenewExpose moves expiry to lifetime seconds from now.
func (s *Shard) RenewExpose(
	ctx context.Context,
	id webapiproto.ExposeID,
	lifetime uint64,
) (plugin.ExposeRecord, error) {
	duration, err := exposeLifetime(lifetime)
	if err != nil {
		return plugin.ExposeRecord{}, err
	}
	value, err := expose.Renew(ctx, s.ExposeShard(), id, duration)
	if err != nil {
		return plugin.ExposeRecord{}, exposeFailure(err)
	}
	return s.portExpose(ctx, value)
}

// RemoveExpose destroys the expose at once.
func (s *Shard) RemoveExpose(ctx context.Context, id webapiproto.ExposeID) error {
	err := expose.Remove(ctx, s.ExposeShard(), id)
	return exposeFailure(err)
}

var (
	_ cloud.Shard          = (*Shard)(nil)
	_ hostaccess.HostShard = (*Shard)(nil)
	_ pluginhost.Shard     = (*Shard)(nil)
)

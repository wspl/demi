package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// Conversations owns the user's slots and host-access workers. Its opaque state
// uses the shard mutex; callers hold no shard lock when calling its methods.
// Construct with NewConversations, do not copy, and close before dependencies.
type Conversations struct {
	mu             *sync.Mutex // The owning shard mutex protects slots, closing and work admission.
	ctx            context.Context
	cancel         context.CancelFunc
	slots          map[webapi.ConversationID]*ConversationSlot
	closing        bool
	work           sync.WaitGroup
	closeOnce      sync.Once
	closed         chan struct{}
	cloudAdmission func(context.Context, cloud.CloudShard, database.DeviceRecord) (*cloudHold, error)
}

// NewConversations creates slots and a worker owner under the shard's lifetime
// context and mutex. Request cancellation never abandons an admitted commit.
func NewConversations(ctx context.Context, mu *sync.Mutex) *Conversations {
	lifetime, cancel := context.WithCancel(ctx)
	return &Conversations{
		mu:     mu,
		ctx:    lifetime,
		cancel: cancel,
		slots:  make(map[webapi.ConversationID]*ConversationSlot),
		closed: make(chan struct{}),
	}
}

// Slot returns the stable opaque slot of the canonical conversation ID.
func (c *Conversations) Slot(id webapi.ConversationID) *ConversationSlot {
	c.mu.Lock()
	defer c.mu.Unlock()
	if slot := c.slots[id]; slot != nil {
		return slot
	}
	slot := &ConversationSlot{
		files:     runners.NewFileGate(id),
		streams:   gates.NewActivity(nil),
		transfers: NewTransferSet(c.mu),
	}
	if c.closing {
		slot.transfers.closings++
	}
	c.slots[id] = slot
	return slot
}

// EndTransfers closes admission, ends registered transfers and streams, and
// waits for their admission release. Each returned hold must be released.
func (c *Conversations) EndTransfers(ctx context.Context) ([]*TransfersClosed, error) {
	c.mu.Lock()
	sets := make([]*TransferSet, 0, len(c.slots))
	for _, slot := range c.slots {
		sets = append(sets, slot.transfers)
	}
	c.mu.Unlock()
	holds := make([]*TransfersClosed, 0, len(sets))
	for _, set := range sets {
		hold, err := set.Close(ctx)
		if err != nil {
			for _, held := range holds {
				held.Release()
			}
			return nil, err
		}
		holds = append(holds, hold)
	}
	return holds, nil
}

// Close stops admission, interrupts streams and transfers, and joins owned
// work, including commits. It must finish before shard dependencies are closed.
func (c *Conversations) Close(ctx context.Context) error {
	c.closeOnce.Do(func() {
		c.mu.Lock()
		c.closing = true
		c.mu.Unlock()
		c.cancel()
		// The close worker has one owner: all Close callers join closed.
		go func() {
			defer close(c.closed)
			holds, _ := c.EndTransfers(context.Background()) // An uncancellable drain cannot fail.
			_ = holds                                        // Keep admission permanently closed during teardown.
			c.work.Wait()
		}()
	})
	select {
	case <-c.closed:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// ConversationSlot owns one conversation's gates and transfer registrations.
// Its methods return synchronized opaque handles, never mutable shard state.
type ConversationSlot struct {
	files     *runners.FileGate
	streams   *gates.Activity
	transfers *TransferSet
	settings  gates.Serial
}

// FileGate is the activity gate other work holds or reserves. Its leases make
// no Host; only host access can acquire the private runner file lease.
func (s *ConversationSlot) FileGate() *gates.Activity {
	return s.files.Gate()
}

// Streams is the activity of admitted user streams.
func (s *ConversationSlot) Streams() *gates.Activity {
	return s.streams
}

// Transfers is the synchronized registry of transfers and user streams.
func (s *ConversationSlot) Transfers() *TransferSet {
	return s.transfers
}

// Settings serializes changes of a conversation's settings.
func (s *ConversationSlot) Settings() *gates.Serial {
	return &s.settings
}

// ConversationHost is the Host reached by an admitted operation. Host must not
// escape the admission; Root is where work starts and Home is the reported home.
type ConversationHost struct {
	Host *remotehost.Host
	Root string
	Home *string
}

// Admitted holds the Host, file lease and Cloud admission of one operation.
// Its owner defers Release and uses Host only while the admission is held.
type Admitted struct {
	Host         ConversationHost
	files        *runners.FileLease
	cloudRelease func()
	perOperation remotehost.Admission
	released     atomic.Bool
	done         func()
}

// Release ends the admission exactly once, releasing files before Cloud.
func (a *Admitted) Release() {
	if a.released.Swap(true) {
		return
	}
	a.files.Release()
	if a.cloudRelease != nil {
		a.cloudRelease()
	}
	if a.done != nil {
		a.done()
	}
}

// HostRole says whether a reachable Host is main or attached.
type HostRole uint8

const (
	// Main is the conversation's selected Host.
	Main HostRole = iota
	// Attached is a device attached to the conversation.
	Attached
)

// ReachableHost names a bound device and the directory its shells start in.
type ReachableHost struct {
	Name   string
	Device webapi.DeviceID
	Path   string
	Role   HostRole
}

// OwnedConversation reads the owner's conversation, regardless of ID case.
// Another user's conversation answers as missing.
func OwnedConversation(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
) (database.ConversationRecord, error) {
	record, err := shard.Control().Conversation(ctx, id)
	if err != nil {
		return database.ConversationRecord{}, &Error{Kind: AccessStorage, Cause: err}
	}
	if record == nil || record.Owner != shard.User() {
		return database.ConversationRecord{}, &Error{Kind: AccessMissing}
	}
	return *record, nil
}

// WithHost runs operation once on the main Host or named bound device, holding
// file and Cloud admission until it returns. Waiting rechecks ownership,
// binding and archive state; dispatched work is never retried.
func WithHost[T any](
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	device *webapi.DeviceID,
	operation func(context.Context, *ConversationHost) (T, error),
) (T, error) {
	admitted, err := AdmitHost(ctx, shard, id, device)
	if err != nil {
		var zero T
		return zero, err
	}
	defer admitted.Release()
	return operation(ctx, &admitted.Host)
}

// AdmitHost takes the conversation's Host admission. It releases the file gate
// before waiting for Cloud and repeats every check after reacquiring it.
// The caller must defer Release on success; ctx cancels admission waits.
func AdmitHost(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	device *webapi.DeviceID,
) (*Admitted, error) {
	owner := shard.Conversations()
	done, err := owner.begin()
	if err != nil {
		return nil, err
	}
	handed := false
	defer func() {
		if !handed {
			done()
		}
	}()
	waitCtx, cancel := context.WithCancel(ctx)
	cancelled := make(chan struct{})
	stop := context.AfterFunc(owner.ctx, func() {
		defer close(cancelled)
		cancel()
	})
	defer func() {
		if !stop() {
			<-cancelled
		}
		cancel()
	}()
	record, err := OwnedConversation(waitCtx, shard, id)
	if err != nil {
		return nil, err
	}
	admitted, err := admitHost(waitCtx, shard, owner, record.ID, device, done)
	if err != nil {
		return nil, err
	}
	handed = true
	return admitted, nil
}

// ResolveTarget resolves metadata without allocating or starting Cloud.
func ResolveTarget(
	ctx context.Context,
	shard HostShard,
	record database.ConversationRecord,
) (database.ExecutionTarget, error) {
	switch target := record.Target.(type) {
	case *webapi.ConversationTargetWorkspace:
		workspace, err := shard.Control().Workspace(ctx, target.WorkspaceID)
		if err != nil {
			return nil, err
		}
		if workspace == nil || workspace.User != record.Owner {
			return nil, &database.Error{
				Kind:   database.Corrupt,
				Table:  "conversations",
				Column: "target_workspace_id",
				Reason: fmt.Sprintf("workspace %s is not one of the owner's", target.WorkspaceID),
			}
		}
		return &database.ExecutionWorkspace{
			WorkspaceID: workspace.ID,
			DeviceID:    workspace.Device,
			Path:        workspace.Path,
		}, nil
	case *webapi.ConversationTargetDevice:
		return &database.ExecutionDevice{DeviceID: target.DeviceID, Path: target.Path}, nil
	case *webapi.ConversationTargetCloud:
		device, err := shard.Control().ManagedDevice(ctx, record.Owner)
		if err != nil {
			return nil, err
		}
		var id *webapi.DeviceID
		home := "/home/demi"
		if device != nil {
			id = &device.ID
			if reported, ok := shard.Devices().Home(device.ID); ok {
				home = reported
			}
		}
		path := home + "/sessions/" + string(record.ID)
		if target.Path != nil {
			path = *target.Path
		}
		return &database.ExecutionCloud{DeviceID: id, Path: path}, nil
	}
	return nil, &database.Error{
		Kind:   database.Corrupt,
		Table:  "conversations",
		Column: "target",
		Reason: "missing target",
	}
}

// ConversationHosts lists the main Host followed by the attached Hosts.
func ConversationHosts(ctx context.Context, shard HostShard, id webapi.ConversationID) ([]ReachableHost, error) {
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		return nil, err
	}
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return nil, &Error{Kind: AccessStorage, Cause: err}
	}
	return reachableHosts(ctx, shard, record, target)
}

// begin registers host-access work before shutdown can start joining it.
func (c *Conversations) begin() (func(), error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closing || c.ctx.Err() != nil {
		return nil, &Error{Kind: AccessCancelled, Cause: context.Canceled}
	}
	c.work.Add(1)
	return c.work.Done, nil
}

// cloudHold keeps the production Cloud admission behind an internal test seam.
type cloudHold struct {
	device    webapi.DeviceID
	operation remotehost.Admission
	release   func()
}

// admitCloud takes Cloud admission without a conversation file lease.
func (c *Conversations) admitCloud(
	ctx context.Context,
	shard cloud.CloudShard,
	device database.DeviceRecord,
) (*cloudHold, error) {
	if c.cloudAdmission != nil {
		return c.cloudAdmission(ctx, shard, device)
	}
	admission, err := cloud.Admit(ctx, shard, device)
	if err != nil {
		return nil, err
	}
	return &cloudHold{device: admission.Device, operation: admission.PerOperation, release: admission.Release}, nil
}

// admit refuses operations after their enclosing conversation admission ends.
func (a *Admitted) admit() (*gates.Lease, error) {
	if a.released.Load() {
		return nil, &host.Error{
			Kind:    host.Unavailable,
			Message: "the conversation's Host changed; the command did not run",
		}
	}
	if a.perOperation != nil {
		return a.perOperation()
	}
	return nil, nil
}

type selectedHost struct {
	device  database.DeviceRecord
	root    string
	prepare bool
}

// selectHost rechecks the authoritative binding after each file-gate wait.
func selectHost(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	named *webapi.DeviceID,
	allocate bool,
) (selectedHost, error) {
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		return selectedHost{}, err
	}
	if record.Archived {
		return selectedHost{}, &Error{Kind: AccessRefused, Cause: Archived}
	}
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return selectedHost{}, &Error{Kind: AccessStorage, Cause: err}
	}
	deviceID := database.ExecutionDeviceID(target)
	root := database.ExecutionPath(target)
	_, prepare := target.(*database.ExecutionCloud)
	if named != nil {
		reachable, err := reachableHosts(ctx, shard, record, target)
		if err != nil {
			return selectedHost{}, err
		}
		bound, err := namedHost(reachable, *named)
		if err != nil {
			return selectedHost{}, err
		}
		deviceID = &bound.Device
		if bound.Role == Attached {
			root = bound.Path
			prepare = false
		}

	}
	var device *database.DeviceRecord
	if deviceID == nil {
		if !allocate {
			return selectedHost{}, &Error{Kind: AccessRefused, Cause: Stopped}
		}
		made, err := cloud.Device(ctx, shard.CloudShard())
		if err != nil {
			return selectedHost{}, &Error{Kind: AccessCloud, Cause: err}
		}
		device = &made
	} else {
		device, err = shard.Control().Device(ctx, *deviceID)
		if err != nil {
			return selectedHost{}, &Error{Kind: AccessStorage, Cause: err}
		}
	}
	if device == nil || device.User != record.Owner {
		return selectedHost{}, &Error{Kind: AccessRefused, Cause: DeviceGone}
	}
	return selectedHost{device: *device, root: root, prepare: prepare}, nil
}

// makeHost binds the private runner file lease to a checked target.
func makeHost(
	shard HostShard,
	files *runners.FileLease,
	selected selectedHost,
	admission remotehost.Admission,
) ConversationHost {
	var home *string
	if value, ok := shard.Devices().Home(selected.device.ID); ok {
		home = &value
	}
	return ConversationHost{
		Host: shard.Devices().ConversationHost(selected.device.ID, files, selected.root, admission),
		Root: selected.root,
		Home: home,
	}
}

// reachableHosts keeps the main-first order used by commands and host routes.
func reachableHosts(
	ctx context.Context,
	shard HostShard,
	record database.ConversationRecord,
	target database.ExecutionTarget,
) ([]ReachableHost, error) {
	var result []ReachableHost
	main := database.ExecutionDeviceID(target)
	if main != nil {
		name := string(*main)
		device, err := shard.Control().Device(ctx, *main)
		if err != nil {
			return nil, &Error{Kind: AccessStorage, Cause: err}
		}
		if device != nil {
			name = device.Name
		}
		result = append(
			result,
			ReachableHost{Name: name, Device: *main, Path: database.ExecutionPath(target), Role: Main},
		)
	}
	attached, err := shard.Control().AttachedHosts(ctx, record.ID)
	if err != nil {
		return nil, &Error{Kind: AccessStorage, Cause: err}
	}
	for _, bound := range attached {
		if main != nil && bound.Device == *main {
			continue
		}
		path, _ := shard.Devices().Home(bound.Device)
		if bound.CWD != nil {
			path = *bound.CWD
		}
		result = append(result, ReachableHost{Name: bound.Name, Device: bound.Device, Path: path, Role: Attached})
	}
	return result, nil
}

// accessError classifies failures from a Host without losing their cause.
func accessError(err error) error {
	if err == nil {
		return nil
	}
	var access *Error
	if errors.As(err, &access) {
		return err
	}
	return &Error{Kind: AccessHost, Cause: err}
}

// prepareAdmittedHost releases a failed admission but leaves the caller to unregister its work.
func prepareAdmittedHost(ctx context.Context, admitted *Admitted, selected selectedHost) error {
	if selected.prepare {
		if err := admitted.Host.Host.FS().
			Mkdir(ctx, selected.root, host.MkdirOptions{Recursive: true}); err != nil {
			admitted.done = nil
			admitted.Release()
			return &Error{Kind: AccessHost, Cause: err}
		}
	}
	return nil
}

func namedHost(reachable []ReachableHost, named webapi.DeviceID) (ReachableHost, error) {
	for _, bound := range reachable {
		if bound.Device == named {
			return bound, nil
		}
	}
	return ReachableHost{}, &Error{Kind: AccessRefused, Cause: NotAttached}
}

// admitHost reacquires the file gate after Cloud admission and transfers both holds on success.
func admitHost(
	waitCtx context.Context,
	shard HostShard,
	owner *Conversations,
	id webapi.ConversationID,
	device *webapi.DeviceID,
	done func(),
) (*Admitted, error) {
	slot := owner.Slot(id)
	var admission *cloudHold
	defer func() {
		if admission != nil {
			admission.release()
		}
	}()
	for {
		files, err := slot.files.Enter(waitCtx, gates.Demand)
		if err != nil {
			return nil, &Error{Kind: AccessCancelled, Cause: err}
		}
		selected, err := selectHost(waitCtx, shard, id, device, true)
		if err != nil {
			files.Release()
			return nil, err
		}
		if selected.device.Kind == webapi.DeviceKindManaged &&
			(admission == nil || admission.device != selected.device.ID) {
			files.Release()
			if admission != nil {
				admission.release()
				admission = nil
			}
			admission, err = owner.admitCloud(waitCtx, shard.CloudShard(), selected.device)
			if err != nil {
				return nil, &Error{Kind: AccessCloud, Cause: err}
			}
			continue
		}
		if selected.device.Kind != webapi.DeviceKindManaged && admission != nil {
			admission.release()
			admission = nil
		}
		admitted := &Admitted{files: files, done: done}
		if admission != nil {
			admitted.cloudRelease = admission.release
			admitted.perOperation = admission.operation
			admission = nil
		}
		admitted.Host = makeHost(shard, files, selected, admitted.admit)
		if err := prepareAdmittedHost(waitCtx, admitted, selected); err != nil {
			return nil, err
		}
		shard.TrackIdle(id)
		return admitted, nil
	}
}

package hostaccess

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// ServiceBinding is the native operation a user stream or one-shot call runs.
type ServiceBinding struct {
	Package   commandwire.PackageDescriptor
	Operation string
}

// UserStreams is the immutable set of published streams pages may open by name.
type UserStreams struct {
	bindings map[string]declare.NativeOperation
	native   *runners.NativeCatalog
}

// StreamDeclaration binds one page stream name to a declared native operation.
type StreamDeclaration struct {
	Name      string
	Operation declare.NativeOperation
}

// NewUserStreams binds declarations served by native; unsupported bindings
// declare nothing. Declaration order determines duplicate-name replacement.
func NewUserStreams(declared []StreamDeclaration, native *runners.NativeCatalog) *UserStreams {
	streams := &UserStreams{bindings: make(map[string]declare.NativeOperation), native: native}
	for _, item := range declared {
		if native.Serves(item.Operation.Package, []string{item.Operation.Operation}) {
			streams.bindings[item.Name] = item.Operation
		}
	}
	return streams
}

// Lookup returns the named binding as an owned value, if declared.
func (s *UserStreams) Lookup(name string) (ServiceBinding, bool) {
	binding, ok := s.bindings[name]
	if !ok {
		return ServiceBinding{}, false
	}
	return ServiceBinding{Package: *s.native.Package(binding.Package), Operation: binding.Operation}, true
}

// UserStream transfers its pipes and lease to the edge, which defers Release.
type UserStream struct {
	// ToHost carries the page's bytes as invocation input.
	ToHost *remotehost.PipeWriter
	// FromHost carries invocation output until completion.
	FromHost *remotehost.PipeReader
	Lease    *Lease
}

// UserCallKind decides whether a call wakes Cloud and counts as activity.
type UserCallKind uint8

const (
	// Starts is new work: ordinary Host demand that wakes Cloud.
	Starts UserCallKind = iota
	// Operates changes running state: activity, but never wakes Cloud.
	Operates
	// Looks reads running state: no activity and never wakes Cloud.
	Looks
)

// ServiceCall carries a native operation and the bound on its JSON answer.
// Args is an object encoded by a generated encoder or contract.EncodeJSON;
// the receiving operation validates it through its generated decoder.
type ServiceCall struct {
	Binding  ServiceBinding
	Args     json.RawMessage
	MaxBytes int
}

// OpenUserStream opens the main Host without waking Cloud. It drops the file
// gate after admission and holds stream activity until completion or revocation.
func OpenUserStream(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	binding ServiceBinding,
) (*UserStream, error) {
	access, waitCtx, stop, err := admitStream(ctx, shard, id, true, true)
	if err != nil {
		return nil, &StreamError{Kind: StreamAccess, Cause: err}
	}
	defer stop()
	handed := false
	defer func() {
		if !handed {
			access.release()
		}
	}()
	request, err := serviceRequest(waitCtx, shard, access.id, access.host, binding, nil)
	if err != nil {
		return nil, &StreamError{Kind: StreamAccess, Cause: err}
	}
	input := shard.Pipes().ToDevice(string(access.device))
	output := shard.Pipes().FromDevice(string(access.device))
	toHost, err := input.Writer()
	if err != nil {
		input.Fail(err.Error())
		output.Fail(err.Error())
		return nil, &StreamError{Kind: StreamFailed, Cause: err}
	}
	fromHost, err := output.Reader()
	if err != nil {
		input.Fail(err.Error())
		output.Fail(err.Error())
		return nil, &StreamError{Kind: StreamFailed, Cause: err}
	}
	service, err := access.host.Host.OpenService(waitCtx, request, input.WireRef(), output.WireRef())
	if err != nil {
		input.Fail("the user stream never opened")
		output.Fail("the user stream never opened")
		var failure *host.Error
		if errors.As(err, &failure) && failure.Kind == host.Offline {
			return nil, &StreamError{Kind: StreamAccess, Cause: accessError(err)}
		}
		return nil, &StreamError{Kind: StreamFailed, Cause: err}
	}
	lease := watchUserStream(access, service, input, output)

	handed = true
	return &UserStream{ToHost: toHost, FromHost: fromHost, Lease: lease}, nil
}

// UserCall returns a one-shot call's JSON answer. Starts holds ordinary Host
// access; other calls register like streams and transitions end them.
func UserCall(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	kind UserCallKind,
	call ServiceCall,
) ([]byte, error) {
	if kind == Starts {
		return startUserCall(ctx, shard, id, call)
	}
	access, waitCtx, stop, err := admitStream(ctx, shard, id, false, kind == Operates)
	if err != nil {
		return nil, &UserCallError{Kind: UserCallAccess, Cause: err}
	}
	defer stop()
	defer access.release()
	request, err := serviceRequest(waitCtx, shard, access.id, access.host, call.Binding, call.Args)
	if err != nil {
		return nil, &UserCallError{Kind: UserCallAccess, Cause: err}
	}
	answer, err := access.host.Host.CallService(waitCtx, request, nil, call.MaxBytes)
	if err != nil {
		return nil, &UserCallError{Kind: UserCallCall, Cause: err}
	}
	return answer, nil
}

type streamAccess struct {
	id       webapi.ConversationID
	device   webapi.DeviceID
	host     ConversationHost
	open     *OpenTransfer
	watching *gates.Lease
	done     func()
}

// release ends stream activity before unregistering its admission.
func (a *streamAccess) release() {
	if a.watching != nil {
		a.watching.Release()
	}
	a.open.Release()
	a.done()
}

// admitStream admits a registered no-wake operation, releasing the file gate
// only after a watching stream has acquired its activity lease.
func admitStream(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	watches, operates bool,
) (*streamAccess, context.Context, func(), error) {
	open, waitCtx, stop, err := registerTransfer(ctx, shard, id)
	if err != nil {
		return nil, nil, nil, err
	}
	done, err := shard.Conversations().begin()
	if err != nil {
		stop()
		open.Release()
		return nil, nil, nil, err
	}
	access := &streamAccess{id: id, open: open, done: done}
	success := false
	defer func() {
		if !success {
			stop()
			access.release()
		}
	}()
	record, err := OwnedConversation(waitCtx, shard, id)
	if err != nil {
		return nil, nil, nil, err
	}
	access.id = record.ID
	slot := shard.Conversations().Slot(record.ID)
	purpose := gates.Maintenance
	if watches || operates {
		purpose = gates.Demand
	}
	files, err := slot.files.Enter(waitCtx, purpose)
	if err != nil {
		return nil, nil, nil, transferError(open, &Error{Kind: AccessCancelled, Cause: err})
	}
	defer files.Release()
	selected, err := streamHost(waitCtx, shard, record.ID)
	if err != nil {
		return nil, nil, nil, err
	}
	access.device = selected.device.ID
	access.host = makeHost(shard, files, selected, func() (*gates.Lease, error) {
		if open.Context().Err() != nil {
			return nil, &host.Error{Kind: host.Interrupted, Message: Busy.Error()}
		}
		return nil, nil
	})
	shard.TrackIdle(record.ID)
	if watches {
		access.watching, err = slot.streams.Enter(waitCtx, gates.Demand)
		if err != nil {
			return nil, nil, nil, transferError(open, err)
		}
	}
	success = true
	return access, waitCtx, stop, nil
}

// serviceRequest binds a user's call to the conversation's directory and catalog.
func serviceRequest(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	admitted ConversationHost,
	binding ServiceBinding,
	args json.RawMessage,
) (remotehost.ServiceRequest, error) {
	commandContext, err := runners.CommandContext(ctx, shard.Control(), shard.User(), id, &commandwire.UserCaller{})
	if err != nil {
		return remotehost.ServiceRequest{}, &Error{Kind: AccessStorage, Cause: err}
	}
	request := remotehost.ServiceRequest{
		Context:   commandContext,
		Package:   binding.Package,
		Operation: binding.Operation,
		Args:      args,
		CWD:       admitted.Root,
		Resolver:  shard.Native().Resolver(shard.PublicURL()),
	}
	if args != nil {
		request.JSON = new(true)
	}
	return request, nil
}

// watchUserStream transfers admission to the completion worker; revoking its lease also ends Done.
func watchUserStream(access *streamAccess, service *remotehost.ServiceStream, input, output *remotehost.Pipe) *Lease {
	lease, _ := NewLease(access.open.Context())
	go func() {
		// Done also ends when the lease context is revoked, so no second watcher is needed.
		_, _ = service.Done(lease.Context()) // Completion is represented by the stream's pipe outcome.
		input.Fail("the user stream ended")
		output.Fail("the user stream ended")
		service.Close()
		lease.Release()
		access.release()
	}()
	return lease
}

func startUserCall(ctx context.Context, shard HostShard, id webapi.ConversationID, call ServiceCall) ([]byte, error) {
	answer, err := WithHost(
		ctx,
		shard,
		id,
		nil,
		func(ctx context.Context, admitted *ConversationHost) ([]byte, error) {
			request, err := serviceRequest(ctx, shard, id, *admitted, call.Binding, call.Args)
			if err != nil {
				return nil, &UserCallError{Kind: UserCallAccess, Cause: err}
			}
			answer, err := admitted.Host.CallService(ctx, request, nil, call.MaxBytes)
			if err != nil {
				return nil, &UserCallError{Kind: UserCallCall, Cause: err}
			}
			return answer, nil
		},
	)
	if err != nil {
		var wrapped *UserCallError
		if errors.As(err, &wrapped) {
			return nil, err
		}
		return nil, &UserCallError{Kind: UserCallAccess, Cause: err}
	}
	return answer, nil
}

func streamHost(ctx context.Context, shard HostShard, id webapi.ConversationID) (selectedHost, error) {
	selected, err := selectHost(ctx, shard, id, nil, false)
	if err != nil {
		return selectedHost{}, err
	}
	if !shard.Devices().Online(selected.device.ID) {
		if selected.device.Kind == webapi.DeviceKindManaged {
			return selectedHost{}, &Error{Kind: AccessRefused, Cause: Stopped}
		}
		return selectedHost{}, accessError(&host.Error{Kind: host.Offline, Message: "The device has no live runner"})
	}
	return selected, nil
}

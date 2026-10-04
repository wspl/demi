package plugin

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// Transport carries one plugin port operation to its answer.
type Transport interface {
	// Request sends a port message and returns its answer.
	Request(context.Context, PortMessage) (PortAnswer, error)
}

// Port is the plugin's side of a request. Its caller's context owns cancellation.
type Port struct{ transport Transport }

// NewPort constructs a port over the supplied transport.
func NewPort(transport Transport) Port { return Port{transport: transport} }

// Forward sends one message, preserving a refusal as a wire answer.
func (p Port) Forward(ctx context.Context, message PortMessage) (PortAnswer, error) {
	return p.transport.Request(ctx, message)
}

// RPC exposes the command operations through the same transport.
func (p Port) RPC() host.RPCPort { return host.NewRPCPort(rpcMessages(p)) }

type rpcMessages struct{ transport Transport }

// Request forwards an RPC request through the plugin port.
func (r rpcMessages) Request(ctx context.Context, request host.PortRequest) (host.PortResponse, error) {
	answer, err := r.transport.Request(ctx, &PortMessageRPC{Request: request})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerRPC); ok {
		return a.Response, nil
	}
	return nil, unexpected("rpc", answer)
}

// ask turns a plugin port refusal into the operation's error.
func (p Port) ask(ctx context.Context, message PortMessage) (PortAnswer, error) {
	answer, err := p.Forward(ctx, message)
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerRefused); ok {
		return nil, a.Refusal
	}
	return answer, nil
}

// Value performs the read_value port operation; ok is false when the
// plugin has no value under key.
func (p Port) Value(ctx context.Context, key string) (StoredValue, bool, error) {
	answer, err := p.ask(ctx, &PortMessageReadValue{Key: key})
	if err != nil {
		return StoredValue{}, false, err
	}
	a, ok := answer.(*PortAnswerValue)
	if !ok {
		return StoredValue{}, false, unexpected("read_value", answer)
	}
	if a.Value == nil {
		return StoredValue{}, false, nil
	}
	return *a.Value, true, nil
}

// Values performs the list_values port operation.
func (p Port) Values(ctx context.Context) (map[string]StoredValue, error) {
	answer, err := p.ask(ctx, &PortMessageListValues{})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerValues); ok {
		return a.Values, nil
	}
	return nil, unexpected("list_values", answer)
}

// WriteValueNaming performs the write_value port operation.
func (p Port) WriteValueNaming(
	ctx context.Context,
	key string,
	value json.RawMessage,
	revision *uint64,
	blobs []core.BlobRef,
) (uint64, error) {
	answer, err := p.ask(ctx, &PortMessageWriteValue{Key: key, Value: value, Revision: revision, Blobs: blobs})
	if err != nil {
		return 0, err
	}
	if a, ok := answer.(*PortAnswerWritten); ok {
		return a.Revision, nil
	}
	return 0, unexpected("write_value", answer)
}

// PutBlob performs the put_blob port operation.
func (p Port) PutBlob(ctx context.Context, bytes core.B64Bytes) (core.BlobRef, error) {
	answer, err := p.ask(ctx, &PortMessagePutBlob{Bytes: bytes})
	if err != nil {
		return "", err
	}
	if a, ok := answer.(*PortAnswerBlob); ok {
		return a.Blob, nil
	}
	return "", unexpected("put_blob", answer)
}

// Blob performs the get_blob port operation; ok is false when the user's namespace lacks the blob.
func (p Port) Blob(ctx context.Context, blob core.BlobRef) (core.B64Bytes, bool, error) {
	answer, err := p.ask(ctx, &PortMessageGetBlob{Blob: blob})
	if err != nil {
		return nil, false, err
	}
	a, ok := answer.(*PortAnswerBytes)
	if !ok {
		return nil, false, unexpected("get_blob", answer)
	}
	if a.Bytes == nil {
		return nil, false, nil
	}
	return *a.Bytes, true, nil
}

// SetDirectories performs the set_directories port operation.
func (p Port) SetDirectories(ctx context.Context, directories []HostDirectory) ([]DirectoryPath, error) {
	answer, err := p.ask(ctx, &PortMessageSetDirectories{Directories: directories})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerDirectories); ok {
		return a.Paths, nil
	}
	return nil, unexpected("set_directories", answer)
}

// ReadHostFiles performs the read_host_files port operation.
func (p Port) ReadHostFiles(ctx context.Context, reads []HostRead) ([]HostFile, error) {
	answer, err := p.ask(ctx, &PortMessageReadHostFiles{Reads: reads})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerHostFiles); ok {
		return a.Files, nil
	}
	return nil, unexpected("read_host_files", answer)
}

// RemoveValue performs the remove_value port operation.
func (p Port) RemoveValue(ctx context.Context, key string, revision uint64) error {
	answer, err := p.ask(ctx, &PortMessageRemoveValue{Key: key, Revision: revision})
	if err != nil {
		return err
	}
	if _, ok := answer.(*PortAnswerDone); ok {
		return nil
	}
	return unexpected("remove_value", answer)
}

// Changed performs the changed port operation.
func (p Port) Changed(ctx context.Context, scope Scope) error {
	answer, err := p.ask(ctx, &PortMessageChanged{Scope: scope})
	if err != nil {
		return err
	}
	if _, ok := answer.(*PortAnswerDone); ok {
		return nil
	}
	return unexpected("changed", answer)
}

// PackageCall performs the package_call port operation.
func (p Port) PackageCall(
	ctx context.Context,
	operation declare.NativeOperation,
	args json.RawMessage,
	kind CallKind,
) (json.RawMessage, error) {
	answer, err := p.ask(ctx, &PortMessagePackageCall{Operation: operation, Args: args, Kind: kind})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerCalled); ok {
		return a.Result, nil
	}
	return nil, unexpected("package_call", answer)
}

// ConversationHosts performs the conversation_hosts port operation.
func (p Port) ConversationHosts(ctx context.Context) ([]ConversationHost, error) {
	answer, err := p.ask(ctx, &PortMessageConversationHosts{})
	if err != nil {
		return nil, err
	}
	if a, ok := answer.(*PortAnswerHosts); ok {
		return a.Hosts, nil
	}
	return nil, unexpected("conversation_hosts", answer)
}

// Exposes performs the list_exposes port operation.
func (p Port) Exposes(ctx context.Context) (ExposeList, error) {
	answer, err := p.ask(ctx, &PortMessageListExposes{})
	if err != nil {
		return ExposeList{}, err
	}
	if a, ok := answer.(*PortAnswerExposes); ok {
		return a.List, nil
	}
	return ExposeList{}, unexpected("list_exposes", answer)
}

// CreateExpose performs the create_expose port operation.
func (p Port) CreateExpose(
	ctx context.Context,
	device webapi.DeviceID,
	address string,
	lifetime uint64,
) (ExposeRecord, error) {
	answer, err := p.ask(ctx, &PortMessageCreateExpose{Device: device, Address: address, Lifetime: lifetime})
	if err != nil {
		return ExposeRecord{}, err
	}
	if a, ok := answer.(*PortAnswerExpose); ok {
		return a.Expose, nil
	}
	return ExposeRecord{}, unexpected("create_expose", answer)
}

// RenewExpose performs the renew_expose port operation.
func (p Port) RenewExpose(ctx context.Context, expose webapi.ExposeID, lifetime uint64) (ExposeRecord, error) {
	answer, err := p.ask(ctx, &PortMessageRenewExpose{Expose: expose, Lifetime: lifetime})
	if err != nil {
		return ExposeRecord{}, err
	}
	if a, ok := answer.(*PortAnswerExpose); ok {
		return a.Expose, nil
	}
	return ExposeRecord{}, unexpected("renew_expose", answer)
}

// RemoveExpose performs the remove_expose port operation.
func (p Port) RemoveExpose(ctx context.Context, expose webapi.ExposeID) error {
	answer, err := p.ask(ctx, &PortMessageRemoveExpose{Expose: expose})
	if err != nil {
		return err
	}
	if _, ok := answer.(*PortAnswerDone); ok {
		return nil
	}
	return unexpected("remove_expose", answer)
}

// WriteValue conditionally writes a value that names no blobs.
func (p Port) WriteValue(ctx context.Context, key string, value json.RawMessage, revision *uint64) (uint64, error) {
	return p.WriteValueNaming(ctx, key, value, revision, nil)
}

// unexpected reports a response belonging to a different port operation.
func unexpected(asked string, answer PortAnswer) error {
	var name string
	switch answer.(type) {
	case *PortAnswerRPC:
		name = "rpc"
	case *PortAnswerValue:
		name = "value"
	case *PortAnswerValues:
		name = "values"
	case *PortAnswerWritten:
		name = "written"
	case *PortAnswerBlob:
		name = "blob"
	case *PortAnswerBytes:
		name = "bytes"
	case *PortAnswerDirectories:
		name = "directories"
	case *PortAnswerHostFiles:
		name = "host_files"
	case *PortAnswerDone:
		name = "done"
	case *PortAnswerCalled:
		name = "called"
	case *PortAnswerHosts:
		name = "hosts"
	case *PortAnswerExposes:
		name = "exposes"
	case *PortAnswerExpose:
		name = "expose"
	case *PortAnswerPanel:
		name = "panel"
	case *PortAnswerPanelRevision:
		name = "panel_revision"
	case *PortAnswerRefused:
		name = "refused"
	}
	return &host.PortError{Kind: host.UnexpectedReply, Asked: asked, Answered: name}
}

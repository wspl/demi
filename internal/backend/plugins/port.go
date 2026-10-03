package plugins

import (
	"context"
	"errors"
	"slices"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// requestPort retains only the plugin host and data identifying the request.
// A plugin's retained work supplies its own context after the original reply.
type requestPort struct {
	user         *User
	index        int
	conversation *webapi.ConversationID
	rpc          *host.RPCPort
	lifetime     context.Context
}

// Request turns product refusals into the plugin contract's refusal answer.
func (p requestPort) Request(ctx context.Context, message plugin.PortMessage) (plugin.PortAnswer, error) {
	operationCtx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(p.lifetime, cancel)
	defer stop()
	defer cancel()
	if err := p.lifetime.Err(); err != nil {
		return nil, &host.PortError{Kind: host.PortEnded, Message: err.Error(), Err: err}
	}
	answer, err := p.answer(operationCtx, message)
	var refusal plugin.PortRefusal
	if errors.As(err, &refusal) {
		return &plugin.PortAnswerRefused{Refusal: refusal}, nil
	}
	return answer, err
}

// answer routes each plugin service to its owner without holding a user mutex.
func (p requestPort) answer(ctx context.Context, message plugin.PortMessage) (plugin.PortAnswer, error) {
	switch m := message.(type) {
	case *plugin.PortMessageRPC:
		return p.forwardRPC(ctx, m)
	case *plugin.PortMessageReadValue:
		return p.readValue(ctx, m)
	case *plugin.PortMessageListValues:
		return p.listValues(ctx)
	case *plugin.PortMessageWriteValue:
		return p.writeValue(ctx, m)
	case *plugin.PortMessageRemoveValue:
		return p.removeValue(ctx, m)
	case *plugin.PortMessagePutBlob:
		return p.putBlob(ctx, m)
	case *plugin.PortMessageGetBlob:
		return p.getBlob(ctx, m)
	case *plugin.PortMessageSetDirectories:
		return p.setDirectories(ctx, m)
	case *plugin.PortMessageReadHostFiles:
		return p.readHostFiles(ctx, m)
	case *plugin.PortMessageChanged:
		return p.markChanged(ctx, m)
	case *plugin.PortMessagePackageCall:
		return p.callPackage(ctx, m)
	case *plugin.PortMessageConversationHosts:
		return p.conversationHosts(ctx)
	case *plugin.PortMessageListExposes:
		return p.listExposes(ctx)
	case *plugin.PortMessageCreateExpose:
		return p.createExpose(ctx, m)
	case *plugin.PortMessageRenewExpose:
		return p.renewExpose(ctx, m)
	case *plugin.PortMessageRemoveExpose:
		return p.removeExpose(ctx, m)
	}
	return nil, &host.PortError{
		Kind:     host.UnexpectedReply,
		Asked:    "port",
		Answered: "no port message",
	}
}

// conversationID refuses conversation-only operations on a user-scoped port.
func (p requestPort) conversationID() (webapi.ConversationID, error) {
	if p.conversation == nil {
		return "", &plugin.PortRefusalNoConversation{}
	}
	return *p.conversation, nil
}

// storageError preserves the storage failure behind the plugin port's error.
func storageError(err error) error {
	return &host.PortError{Kind: host.PortFailed, Message: err.Error(), Err: err}
}

// readValue serves the plugin’s ReadValue storage request.
func (p requestPort) readValue(
	ctx context.Context,
	m *plugin.PortMessageReadValue,
) (plugin.PortAnswer, error) {
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	value, found, err := u.control.PluginValue(ctx, u.id, id, m.Key)
	if err != nil {
		return nil, storageError(err)
	}
	var stored *plugin.StoredValue
	if found {
		stored = &plugin.StoredValue{Value: value.Document, Revision: value.Revision}
	}
	return &plugin.PortAnswerValue{Value: stored}, nil
}

// listValues serves the plugin’s ListValues storage request.
func (p requestPort) listValues(ctx context.Context) (plugin.PortAnswer, error) {
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	values, err := u.control.PluginValues(ctx, u.id, id)
	if err != nil {
		return nil, storageError(err)
	}
	stored := make(map[string]plugin.StoredValue, len(values))
	for key, value := range values {
		stored[key] = plugin.StoredValue{Value: value.Document, Revision: value.Revision}
	}
	return &plugin.PortAnswerValues{Values: stored}, nil
}

// writeValue serves the plugin’s WriteValue storage request.
func (p requestPort) writeValue(
	ctx context.Context,
	m *plugin.PortMessageWriteValue,
) (plugin.PortAnswer, error) {
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	revision, err := u.control.WritePluginValue(
		context.WithoutCancel(ctx),
		database.ValueWrite{
			User:     u.id,
			Plugin:   id,
			Key:      m.Key,
			Document: m.Value,
			Revision: m.Revision,
			Blobs:    append([]core.BlobRef{}, m.Blobs...),
		},
		u.shard.BlobUses(),
	)
	if errors.Is(err, database.ErrRevisionConflict) {
		return nil, &plugin.PortRefusalConflict{}
	}
	if err != nil {
		return nil, storageError(err)
	}
	return &plugin.PortAnswerWritten{Revision: revision}, nil
}

// removeValue serves the plugin’s RemoveValue storage request.
func (p requestPort) removeValue(
	ctx context.Context,
	m *plugin.PortMessageRemoveValue,
) (plugin.PortAnswer, error) {
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	err := u.control.RemovePluginValue(
		context.WithoutCancel(ctx),
		u.id,
		id,
		m.Key,
		m.Revision,
		u.shard.BlobUses(),
	)
	if errors.Is(err, database.ErrRevisionConflict) {
		return nil, &plugin.PortRefusalConflict{}
	}
	if err != nil {
		return nil, storageError(err)
	}
	return &plugin.PortAnswerDone{}, nil
}

// setDirectories serves the plugin’s SetDirectories storage request.
func (p requestPort) setDirectories(
	ctx context.Context,
	m *plugin.PortMessageSetDirectories,
) (plugin.PortAnswer, error) {
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	if err := plugin.CheckDirectories(m.Directories); err != nil {
		return nil, storageError(err)
	}
	if err := u.control.SetPluginDirectories(
		context.WithoutCancel(ctx),
		u.id,
		id,
		m.Directories,
		u.shard.BlobUses(),
	); err != nil {
		return nil, storageError(err)
	}
	paths := make([]plugin.DirectoryPath, 0, len(m.Directories))
	for _, directory := range m.Directories {
		path := directory.Path(plugin.ID(id))
		paths = append(paths, plugin.DirectoryPath{Name: directory.Name, Path: path})
	}
	return &plugin.PortAnswerDirectories{Paths: paths}, nil
}

// forwardRPC serves the plugin’s RPC port request.
func (p requestPort) forwardRPC(ctx context.Context, m *plugin.PortMessageRPC) (plugin.PortAnswer, error) {
	if p.rpc == nil {
		return nil, &host.PortError{
			Kind:     host.UnexpectedReply,
			Asked:    "rpc",
			Answered: "no rpc port: the request is not a command",
		}
	}
	response, err := p.rpc.Forward(ctx, m.Request)
	return &plugin.PortAnswerRPC{Response: response}, err
}

// callPackage serves the plugin’s PackageCall port request.
func (p requestPort) callPackage(
	ctx context.Context,
	m *plugin.PortMessagePackageCall,
) (plugin.PortAnswer, error) {
	u := p.user
	if !slices.Contains(u.registry.plugins[p.index].packages, m.Operation.Package) {
		return nil, &host.PortError{
			Kind:     host.UnexpectedReply,
			Asked:    "package_call",
			Answered: "a package the plugin's commands do not bind",
		}
	}
	conversation, err := p.conversationID()
	if err != nil {
		return nil, err
	}
	result, err := u.shard.PackageCall(ctx, conversation, m.Operation, m.Args, m.Kind)
	return &plugin.PortAnswerCalled{Result: result}, err
}

// markChanged serves the plugin’s Changed port request.
func (p requestPort) markChanged(_ context.Context, m *plugin.PortMessageChanged) (plugin.PortAnswer, error) {
	u := p.user
	if m.Scope == plugin.ScopeConversation {
		if _, err := p.conversationID(); err != nil {
			return nil, err
		}
		u.changed(p.index, p.conversation)
	} else {
		u.changed(p.index, nil)
	}
	return &plugin.PortAnswerDone{}, nil
}

// putBlob stores plugin blob bytes.
func (p requestPort) putBlob(ctx context.Context, m *plugin.PortMessagePutBlob) (plugin.PortAnswer, error) {
	blob, err := p.user.shard.PutBlob(ctx, m.Bytes)
	return &plugin.PortAnswerBlob{Blob: blob}, err
}

// getBlob reads plugin blob bytes.
func (p requestPort) getBlob(ctx context.Context, m *plugin.PortMessageGetBlob) (plugin.PortAnswer, error) {
	bytes, err := p.user.shard.Blob(ctx, m.Blob)
	return &plugin.PortAnswerBytes{Bytes: bytes}, err
}

// readHostFiles reads files from the request’s conversation hosts.
func (p requestPort) readHostFiles(ctx context.Context, m *plugin.PortMessageReadHostFiles) (plugin.PortAnswer, error) {
	conversation, err := p.conversationID()
	if err != nil {
		return nil, err
	}
	files, err := p.user.shard.ReadHostFiles(ctx, conversation, m.Reads)
	return &plugin.PortAnswerHostFiles{Files: files}, err
}

// conversationHosts lists hosts reachable from the request’s conversation.
func (p requestPort) conversationHosts(ctx context.Context) (plugin.PortAnswer, error) {
	conversation, err := p.conversationID()
	if err != nil {
		return nil, err
	}
	hosts, err := p.user.shard.ConversationHosts(ctx, conversation)
	return &plugin.PortAnswerHosts{Hosts: hosts}, err
}

// listExposes lists the user’s available exposes.
func (p requestPort) listExposes(ctx context.Context) (plugin.PortAnswer, error) {
	list, err := p.user.shard.Exposes(ctx)
	return &plugin.PortAnswerExposes{List: list}, err
}

// createExpose creates an expose through the shard.
func (p requestPort) createExpose(ctx context.Context, m *plugin.PortMessageCreateExpose) (plugin.PortAnswer, error) {
	expose, err := p.user.shard.CreateExpose(ctx, m.Device, m.Address, m.Lifetime)
	return &plugin.PortAnswerExpose{Expose: expose}, err
}

// renewExpose renews an expose through the shard.
func (p requestPort) renewExpose(ctx context.Context, m *plugin.PortMessageRenewExpose) (plugin.PortAnswer, error) {
	expose, err := p.user.shard.RenewExpose(ctx, m.Expose, m.Lifetime)
	return &plugin.PortAnswerExpose{Expose: expose}, err
}

// removeExpose removes an expose through the shard.
func (p requestPort) removeExpose(ctx context.Context, m *plugin.PortMessageRemoveExpose) (plugin.PortAnswer, error) {
	if err := p.user.shard.RemoveExpose(ctx, m.Expose); err != nil {
		return nil, err
	}
	return &plugin.PortAnswerDone{}, nil
}

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
	u := p.user
	id := string(u.registry.plugins[p.index].manifest.ID)
	switch m := message.(type) {
	case *plugin.PortMessageRPC:
		if p.rpc == nil {
			return nil, &host.PortError{Kind: host.UnexpectedReply, Asked: "rpc", Answered: "no rpc port: the request is not a command"}
		}
		response, err := p.rpc.Forward(ctx, m.Request)
		return &plugin.PortAnswerRPC{Response: response}, err
	case *plugin.PortMessageReadValue:
		value, err := u.control.PluginValue(ctx, u.id, id, m.Key)
		if err != nil {
			return nil, storageError(err)
		}
		var stored *plugin.StoredValue
		if value != nil {
			stored = &plugin.StoredValue{Value: value.Document, Revision: value.Revision}
		}
		return &plugin.PortAnswerValue{Value: stored}, nil
	case *plugin.PortMessageListValues:
		values, err := u.control.PluginValues(ctx, u.id, id)
		if err != nil {
			return nil, storageError(err)
		}
		stored := make(map[string]plugin.StoredValue, len(values))
		for key, value := range values {
			stored[key] = plugin.StoredValue{Value: value.Document, Revision: value.Revision}
		}
		return &plugin.PortAnswerValues{Values: stored}, nil
	case *plugin.PortMessageWriteValue:
		written, err := u.control.WritePluginValue(context.WithoutCancel(ctx), database.ValueWrite{User: u.id, Plugin: id, Key: m.Key, Document: m.Value, Revision: m.Revision, Blobs: append([]core.BlobRef{}, m.Blobs...)}, u.shard.BlobUses())
		if err != nil {
			return nil, storageError(err)
		}
		return writtenAnswer(written, false)
	case *plugin.PortMessageRemoveValue:
		written, err := u.control.RemovePluginValue(context.WithoutCancel(ctx), u.id, id, m.Key, m.Revision, u.shard.BlobUses())
		if err != nil {
			return nil, storageError(err)
		}
		return writtenAnswer(written, true)
	case *plugin.PortMessagePutBlob:
		blob, err := u.shard.PutBlob(ctx, m.Bytes)
		return &plugin.PortAnswerBlob{Blob: blob}, err
	case *plugin.PortMessageGetBlob:
		bytes, err := u.shard.Blob(ctx, m.Blob)
		return &plugin.PortAnswerBytes{Bytes: bytes}, err
	case *plugin.PortMessageSetDirectories:
		if err := plugin.CheckDirectories(m.Directories); err != nil {
			return nil, storageError(err)
		}
		if err := u.control.SetPluginDirectories(context.WithoutCancel(ctx), u.id, id, m.Directories, u.shard.BlobUses()); err != nil {
			return nil, storageError(err)
		}
		paths := make([]plugin.DirectoryPath, 0, len(m.Directories))
		for _, directory := range m.Directories {
			paths = append(paths, plugin.DirectoryPath{Name: directory.Name, Path: directory.Path(plugin.ID(id))})
		}
		return &plugin.PortAnswerDirectories{Paths: paths}, nil
	case *plugin.PortMessageReadHostFiles:
		conversation, err := p.conversationID()
		if err != nil {
			return nil, err
		}
		files, err := u.shard.ReadHostFiles(ctx, conversation, m.Reads)
		return &plugin.PortAnswerHostFiles{Files: files}, err
	case *plugin.PortMessageChanged:
		if m.Scope == plugin.ScopeConversation {
			if _, err := p.conversationID(); err != nil {
				return nil, err
			}
			u.changed(p.index, p.conversation)
		} else {
			u.changed(p.index, nil)
		}
		return &plugin.PortAnswerDone{}, nil
	case *plugin.PortMessagePackageCall:
		if !slices.Contains(u.registry.plugins[p.index].packages, m.Operation.Package) {
			return nil, &host.PortError{Kind: host.UnexpectedReply, Asked: "package_call", Answered: "a package the plugin's commands do not bind"}
		}
		conversation, err := p.conversationID()
		if err != nil {
			return nil, err
		}
		result, err := u.shard.PackageCall(ctx, conversation, m.Operation, m.Args, m.Kind)
		return &plugin.PortAnswerCalled{Result: result}, err
	case *plugin.PortMessageConversationHosts:
		conversation, err := p.conversationID()
		if err != nil {
			return nil, err
		}
		hosts, err := u.shard.ConversationHosts(ctx, conversation)
		return &plugin.PortAnswerHosts{Hosts: hosts}, err
	case *plugin.PortMessageListExposes:
		list, err := u.shard.Exposes(ctx)
		return &plugin.PortAnswerExposes{List: list}, err
	case *plugin.PortMessageCreateExpose:
		expose, err := u.shard.CreateExpose(ctx, m.Device, m.Address, m.Lifetime)
		return &plugin.PortAnswerExpose{Expose: expose}, err
	case *plugin.PortMessageRenewExpose:
		expose, err := u.shard.RenewExpose(ctx, m.Expose, m.Lifetime)
		return &plugin.PortAnswerExpose{Expose: expose}, err
	case *plugin.PortMessageRemoveExpose:
		if err := u.shard.RemoveExpose(ctx, m.Expose); err != nil {
			return nil, err
		}
		return &plugin.PortAnswerDone{}, nil
	}
	return nil, &host.PortError{Kind: host.UnexpectedReply, Asked: "port", Answered: "no port message"}
}

// conversationID refuses conversation-only operations on a user-scoped port.
func (p requestPort) conversationID() (webapi.ConversationID, error) {
	if p.conversation == nil {
		return "", &plugin.PortRefusalNoConversation{}
	}
	return *p.conversation, nil
}

// writtenAnswer preserves database compare-and-set conflicts as plugin refusals.
func writtenAnswer(written database.Written, removed bool) (plugin.PortAnswer, error) {
	switch w := written.(type) {
	case *database.WrittenRevision:
		if removed {
			return &plugin.PortAnswerDone{}, nil
		}
		return &plugin.PortAnswerWritten{Revision: w.Revision}, nil
	case *database.WrittenConflict:
		return nil, &plugin.PortRefusalConflict{}
	case *database.WrittenRefused:
		return nil, storageError(w.Err)
	}
	return nil, &host.PortError{Kind: host.PortFailed, Message: "no plugin write result"}
}

// storageError preserves the storage failure behind the plugin port's error.
func storageError(err error) error {
	return &host.PortError{Kind: host.PortFailed, Message: err.Error(), Err: err}
}

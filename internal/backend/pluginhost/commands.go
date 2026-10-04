package pluginhost

import (
	"context"
	"errors"
	"slices"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

type forward struct {
	user  *User
	index int
	strip int
}

// Call forwards an RPC command with its path relative to the plugin's tree.
func (f forward) Call(ctx context.Context, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	if f.user == nil {
		return 0, &host.RPCError{
			Kind:    host.HandlerFailed,
			Message: "no plugin instance serves the startup check",
		}
	}
	invocation.Path = slices.Clone(invocation.Path[f.strip:])
	var conversation *webapiproto.ConversationID
	if id, err := webapiproto.ParseConversationID(string(invocation.Context.Conversation)); err == nil {
		conversation = &id
	}
	reply, err := f.user.request(
		ctx,
		f.index,
		&plugin.RequestCommand{User: f.user.id, Invocation: invocation},
		conversation,
		&port,
	)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return 0, &host.PortError{Kind: host.PortEnded, Message: err.Error(), Err: err}
		}
		translated := plugin.RPCError(plugin.RequestError(err))
		// Keep the original cause even when it is not already a plugin.Error.
		switch failure := translated.(type) {
		case *host.RPCError:
			failure.Err = err
		case *host.PortError:
			failure.Err = err
		}
		return 0, translated
	}
	if exit, ok := reply.(*plugin.ReplyExit); ok {
		return exit.Code, nil
	}
	failure := wrongReply("a command", reply)
	return 0, plugin.RPCError(plugin.RequestError(failure))
}

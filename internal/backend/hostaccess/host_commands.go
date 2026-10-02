package hostaccess

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// HostGroup declares demi host list, current and shell with the Rust help text.
// Its handlers fail after the shard's Conversations owner has closed.
func HostGroup(shard HostShard) host.Declared { panic("not written: b-hostaccess") }

// InvocationConversation is the conversation the invoking job belongs to.
func InvocationConversation(invocation host.RPCInvocation) (webapi.ConversationID, error) {
	panic("not written: b-hostaccess")
}

// Reachable returns the calling conversation's Hosts, with RPC errors.
func Reachable(ctx context.Context, shard HostShard, id webapi.ConversationID) ([]ReachableHost, error) {
	panic("not written: b-hostaccess")
}

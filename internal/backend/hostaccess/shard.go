package hostaccess

import (
	"context"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// HostShard supplies the handles host access needs and the idle watch every
// Host admission starts. Handles remain stable until Conversations.Close ends.
// Callbacks synchronize their own state and run without the shard mutex held.
// No callback exposes mutable shard state. Conversations and PluginInstalls
// use the same mutex as their owning shard.
type HostShard interface {
	// User identifies the owner.
	User() webapi.UserID
	// Control supplies durable control records.
	Control() *database.ControlService
	// Clock is the wall clock the backend reads times from.
	Clock() core.Clock
	// Devices is the user's devices, each with its runner connection.
	Devices() *runners.Devices
	// Pipes is the pipes of the user's devices.
	Pipes() *remotehost.Pipes
	// Commands is each agent node's commands, for the rpc calls of its jobs.
	Commands() *runners.CommandRouter
	// Conversations owns each conversation's slot, file gate and transfers.
	Conversations() *Conversations
	// Blobs is the user's blob namespace.
	Blobs() *blobs.Namespace
	// ConversationDB is the database of the user's conversation.
	ConversationDB(conversation webapi.ConversationID) *database.ConversationDB
	// Native is the command packages the conversations' commands bind to.
	Native() *runners.NativeCatalog
	// PublicURL is where runners fetch the packages' executables from.
	PublicURL() *runners.PublicURL
	// CloudShard is the shard as the user's Cloud sees it.
	CloudShard() cloud.Shard
	// TrackIdle starts the conversation's idle watch unless one runs.
	TrackIdle(conversation webapi.ConversationID)
	// DirectorySets is the Host directories of the user's plugins.
	DirectorySets(ctx context.Context) (DirectorySets, error)
	// PluginInstalls remembers the Hosts' installed directories.
	PluginInstalls() *PluginInstalls
	// JobEnded reports a finished or stopped job, which may change plugin views.
	JobEnded(conversation webapi.ConversationID)
}

// RootOf names the root node in the spelling the conversation index keeps.
func RootOf(conversation webapi.ConversationID) core.NodeID {
	return core.NodeID(conversation)
}

// ConversationOf names the conversation of a root opened by RootOf.
func ConversationOf(root core.NodeID) webapi.ConversationID {
	return webapi.ConversationID(root)
}

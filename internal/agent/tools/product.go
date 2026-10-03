package tools

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// Profile describes the overrides a child takes from a named subagent profile.
// The shared contract remains defined and generated in core.
type Profile = core.Profile

// Toolset supplies the commands and profiles a tree takes when it opens and
// keeps until it closes. Revision distinguishes changed commands or profiles.
// Callers treat its commands and profiles as immutable.
type Toolset struct {
	// Commands contains the tree's offered commands.
	Commands *host.CommandSet
	// Profiles contains its named subagent profiles.
	Profiles []Profile
	// Revision identifies the command and profile revision.
	Revision string
}

// ToolsetSource supplies the product's current toolset when a tree opens.
type ToolsetSource interface {
	Current(ctx context.Context) (Toolset, error)
}

// Current returns this fixed toolset.
func (t Toolset) Current(_ context.Context) (Toolset, error) { return t, nil }

// NodeContext identifies the node a product question concerns.
type NodeContext struct {
	// Node is the node whose request or tool call it is.
	Node core.NodeID
	// Root owns the execution target shared by every node of this tree.
	Root core.NodeID
	// CWD is the node's working directory.
	CWD string
}

// HostResolver resolves a node's current execution target through the
// conversation's host access. Equal targets must have equal Host keys.
// H preserves the product's concrete Host type for its environment factory.
type HostResolver[H host.Host] interface {
	Host(ctx context.Context, node NodeContext) (H, error)
}

// ContextSource supplies what a model must learn before a request.
type ContextSource interface {
	// Name is recorded on the source's blocks: execution, or a plugin's id.
	Name() string
	// Context returns new text, or nil. Seen contains this source's own
	// replayed blocks, oldest first. A failure adds no block; the source is
	// asked again before the next request.
	Context(ctx context.Context, node NodeContext, turn core.TurnID, seen []string) (*string, error)
}

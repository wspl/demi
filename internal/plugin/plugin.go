package plugin

import (
	"context"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
)

// Factory carries the immutable manifest and creates one instance per user.
// Implementations must be safe for concurrent calls.
type Factory interface {
	Manifest() Manifest
	Instance() Plugin
}

// Plugin handles one user's requests through their ports.
type Plugin interface {
	Call(context.Context, Request, Port) (Reply, error)
}

// CommandPlugin serves its declared rpc commands through the command port.
type CommandPlugin struct {
	commands  host.CommandSet
	placement Placement
}

// NewCommandPlugin registers the trees in the placement used by the plugin host.
func NewCommandPlugin(placement Placement, trees []host.Declared) (*CommandPlugin, error) {
	p := &CommandPlugin{placement: placement}
	if placement == PlacementDemi {
		trees = []host.Declared{host.Group(DemiRoot, DemiSummary, trees...)}
	}
	for _, tree := range trees {
		if err := p.commands.Register(tree); err != nil {
			return nil, err
		}
	}
	return p, nil
}

// ManifestCommands returns the plugin's trees in their declared placement.
func (p *CommandPlugin) ManifestCommands() []Commands {
	trees := p.commands.Declarations()
	if p.placement == PlacementDemi {
		trees = trees[0].(*declare.Group[declare.NativeOperation]).Subcommands
	}
	commands := make([]Commands, 0, len(trees))
	for _, tree := range trees {
		commands = append(commands, Commands{Placement: p.placement, Tree: Declaration{Node: tree}})
	}
	return commands
}

// invocation prefixes the platform root without changing the caller's path.
func (p *CommandPlugin) invocation(invocation host.RPCInvocation) host.RPCInvocation {
	if p.placement == PlacementDemi {
		invocation.Path = append([]string{DemiRoot}, invocation.Path...)
	}
	return invocation
}

// Command dispatches a path relative to the plugin's own tree.
func (p *CommandPlugin) Command(ctx context.Context, invocation host.RPCInvocation, port Port) (Reply, error) {
	code, err := p.commands.Dispatch(ctx, p.invocation(invocation), port.RPC())
	if err != nil {
		return nil, RequestError(err)
	}
	return &ReplyExit{Code: code}, nil
}

// Check validates an invocation whose handler the plugin answers directly.
func (p *CommandPlugin) Check(invocation host.RPCInvocation) error {
	_, err := p.commands.Check(p.invocation(invocation))
	if err != nil {
		return RequestError(err)
	}
	return nil
}

// Call dispatches a command and refuses undeclared page or context requests.
func (p *CommandPlugin) Call(ctx context.Context, request Request, port Port) (Reply, error) {
	switch r := request.(type) {
	case *RequestCommand:
		return p.Command(ctx, r.Invocation, port)
	case *RequestPageState, *RequestPageCall:
		return nil, Undeclared("page")
	case *RequestContext:
		return nil, Undeclared("context source")
	}
	return nil, Undeclared("request")
}

// PortHandled marks an rpc leaf the plugin handles with its whole port.
type PortHandled struct{}

// Call refuses dispatch because the plugin must handle this leaf itself.
func (PortHandled) Call(_ context.Context, invocation host.RPCInvocation, _ host.RPCPort) (uint8, error) {
	return 0, &host.RPCError{Kind: host.HandlerFailed, Message: fmt.Sprintf("\"%s\" is answered by its plugin, not dispatched", strings.Join(invocation.Path, " "))}
}

// NoRequests is the instance for a plugin that receives no requests.
type NoRequests struct{}

// Call refuses every request to this identity-only plugin.
func (NoRequests) Call(_ context.Context, request Request, _ Port) (Reply, error) {
	var what string
	switch request.(type) {
	case *RequestCommand:
		what = "command"
	case *RequestContext:
		what = "context source"
	case *RequestPageState:
		what = "page"
	case *RequestPageCall:
		what = "page"
	}
	return nil, Undeclared(what)
}

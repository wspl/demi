package backendtest

import (
	"context"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
)

// CommandProbe prints the invoking user and the plugin-relative command path.
// Its manifest is configurable for assembled registry scenarios.
type CommandProbe struct{ Declaration plugin.Manifest }

// Manifest returns the scenario's declarations.
func (p *CommandProbe) Manifest() plugin.Manifest { return p.Declaration }

// Instance creates the stateless command printer.
func (*CommandProbe) Instance() plugin.Plugin { return commandPrinter{} }

type commandPrinter struct{}

func (commandPrinter) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	call, ok := request.(*plugin.RequestCommand)
	if !ok {
		return nil, &plugin.ErrorFailed{Message: "the probe declares only commands"}
	}
	err := port.RPC().Stdout(ctx, []byte(fmt.Sprintf("%s: %s", call.User, strings.Join(call.Invocation.Path, " "))))
	return &plugin.ReplyExit{}, err
}

// ProbeCommand declares a single run leaf, RPC unless a native operation is given.
func ProbeCommand(name string, placement plugin.Placement, operation *declare.NativeOperation) plugin.Commands {
	var kind declare.LeafKind[declare.NativeOperation] = &declare.RPC[declare.NativeOperation]{}
	summary := "A group."
	if operation != nil {
		kind = &declare.Native[declare.NativeOperation]{Binding: *operation}
		summary = "A native group."
	}
	return plugin.Commands{Placement: placement, Tree: plugin.Declaration{Node: &declare.Group[declare.NativeOperation]{Name: name, Summary: summary, Subcommands: []declare.Node[declare.NativeOperation]{&declare.Leaf[declare.NativeOperation]{Name: "run", Summary: "Run.", Kind: kind}}}}}
}

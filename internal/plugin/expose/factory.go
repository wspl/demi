package expose

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/plugin"
)

// Factory declares the expose commands and page and creates each user's instance.
// Its compiled declarations are immutable and safe for concurrent use.
type Factory struct {
	id       plugin.ID
	commands *plugin.CommandPlugin
	state    *cmddecl.Schema
	params   *cmddecl.Schema
	result   *cmddecl.Schema
}

// New constructs the expose plugin's factory from its generated contracts.
func New() (*Factory, error) {
	id, err := plugin.ParseID("expose")
	if err != nil {
		return nil, fmt.Errorf("declare expose identity: %w", err)
	}
	commands, err := commandSet()
	if err != nil {
		return nil, fmt.Errorf("declare expose commands: %w", err)
	}
	state, err := cmddecl.NewSchema(ExposeStatePluginJSONSchema())
	if err != nil {
		return nil, fmt.Errorf("declare expose state: %w", err)
	}
	params, err := cmddecl.NewSchema(ExposeCallPluginJSONSchema())
	if err != nil {
		return nil, fmt.Errorf("declare expose calls: %w", err)
	}
	result, err := cmddecl.NewSchema(json.RawMessage(`{"title":"null","type":"null"}`))
	if err != nil {
		return nil, fmt.Errorf("declare expose result: %w", err)
	}
	return &Factory{id: id, commands: commands, state: state, params: params, result: result}, nil
}

// Manifest returns independent declaration containers sharing only immutable schemas.
func (f *Factory) Manifest() plugin.Manifest {
	return plugin.Manifest{
		ID: f.id, Name: "Host expose",
		Description: "Gives a service on one of your hosts a public URL for an hour, with `demi expose`.",
		Commands:    f.commands.ManifestCommands(),
		Page: &plugin.Page{
			PanelKinds: []string{"page"},
			Package:    "@demicodes/plugin-expose",
			User: &plugin.State{
				Schema: plugin.Schema{Schema: f.state},
				Topics: []plugin.Topic{plugin.TopicExposes},
			},
			Methods: []plugin.Method{
				{
					Name:   "renew",
					Scope:  plugin.ScopeUser,
					Params: plugin.Schema{Schema: f.params},
					Result: plugin.Schema{Schema: f.result},
				},
				{
					Name:   "remove",
					Scope:  plugin.ScopeUser,
					Params: plugin.Schema{Schema: f.params},
					Result: plugin.Schema{Schema: f.result},
				},
			},
		},
	}
}

// Instance creates one user's stateless handler; its port owns persisted numbers.
func (f *Factory) Instance() plugin.Plugin {
	return &instance{commands: f.commands}
}

type instance struct{ commands *plugin.CommandPlugin }

// Call dispatches expose commands and page requests.
func (i *instance) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	switch r := request.(type) {
	case *plugin.RequestCommand:
		if err := i.commands.Check(r.Invocation); err != nil {
			return nil, err
		}
		code, err := run(ctx, r.Invocation, port)
		if err != nil {
			return nil, plugin.RequestError(err)
		}
		return &plugin.ReplyExit{Code: code}, nil
	case *plugin.RequestPageState:
		value, err := state(ctx, port)
		if err != nil {
			return nil, plugin.RequestError(err)
		}
		return &plugin.ReplyState{State: value}, nil
	case *plugin.RequestPageCall:
		value, err := pageCall(ctx, r.Method, r.Params, port)
		if err != nil {
			return nil, plugin.RequestError(err)
		}
		return &plugin.ReplyResult{Result: value}, nil
	case *plugin.RequestPanelTab:
		return &plugin.ReplyDone{}, nil
	case *plugin.RequestTopic:
		return nil, plugin.Undeclared("topic")
	case *plugin.RequestContext:
		return nil, plugin.Undeclared("context source")
	}
	return nil, plugin.Undeclared("request")
}

var (
	_ plugin.Factory = (*Factory)(nil)
	_ plugin.Plugin  = (*instance)(nil)
)

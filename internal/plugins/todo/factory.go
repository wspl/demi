package todo

import (
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// Factory declares the todo commands over node command storage.
type Factory struct{ commands *plugin.CommandPlugin }

// New constructs and validates the todo command declarations.
func New() (*Factory, error) {
	var children []host.Declared
	list, err := listCommand()
	if err != nil {
		return nil, err
	}
	children = append(children, list)
	changedOutput, err := declare.NewSchema(OneTodoJSONSchema())
	if err != nil {
		return nil, err
	}
	addInput, err := declare.NewSchema(AddArgsJSONSchema())
	if err != nil {
		return nil, err
	}
	children = append(children, host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "add",
		Summary:       "Add a new todo.",
		SuccessOutput: new("writes the created todo as raw text, or JSON matching { todo } when --json is passed"),
		FailureOutput: new("writes validation or storage errors to stderr and exits non-zero"),
		Input:         addInput,
		Positionals:   &[]string{"text"},
		Output:        &declare.LeafOutput{JSON: changedOutput},
		Kind:          &declare.RPC[declare.NativeOperation]{},
	}, host.TypedRPC(DecodeAddArgs, add)))
	updateInput, err := declare.NewSchema(UpdateArgsJSONSchema())
	if err != nil {
		return nil, err
	}
	children = append(children, host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "update",
		Summary:       "Update todo text or status.",
		SuccessOutput: new("writes the updated todo as raw text, or JSON matching { todo } when --json is passed"),
		FailureOutput: new("writes \"Todo not found\" or validation/storage errors to stderr and exits non-zero"),
		Input:         updateInput,
		Positionals:   &[]string{"id"},
		Output:        &declare.LeafOutput{JSON: changedOutput},
		Kind:          &declare.RPC[declare.NativeOperation]{},
	}, host.TypedRPC(DecodeUpdateArgs, update)))
	doneInput, err := declare.NewSchema(DoneArgsJSONSchema())
	if err != nil {
		return nil, err
	}
	children = append(children, host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "done",
		Summary:       "Mark a todo as done.",
		SuccessOutput: new("writes the completed todo as raw text, or JSON matching { todo } when --json is passed"),
		FailureOutput: new("writes \"Todo not found\" or validation/storage errors to stderr and exits non-zero"),
		Input:         doneInput,
		Positionals:   &[]string{"id"},
		Output:        &declare.LeafOutput{JSON: changedOutput},
		Kind:          &declare.RPC[declare.NativeOperation]{},
	}, host.TypedRPC(DecodeDoneArgs, done)))
	commands, err := plugin.NewCommandPlugin(
		plugin.PlacementDemi,
		[]host.Declared{host.Group("todo", "Manage an agent-session-scoped task list for coding work.", children...)},
	)
	if err != nil {
		return nil, err
	}
	return &Factory{commands: commands}, nil
}

// Manifest returns the plugin's declarations.
func (f *Factory) Manifest() plugin.Manifest {
	return plugin.Manifest{
		ID:          "todo",
		Name:        "Todo list",
		Description: "A task list the agent keeps for each conversation, with `demi todo`.",
		Commands:    f.commands.ManifestCommands(),
	}
}

// Instance returns the stateless todo command dispatcher.
func (f *Factory) Instance() plugin.Plugin { return f.commands }

func listCommand() (host.Declared, error) {
	listOutput, err := declare.NewSchema(ListJSONSchema())
	if err != nil {
		return host.Declared{}, err
	}
	return host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:    "list",
		Summary: "List todos for the current agent session.",
		SuccessOutput: new(
			"writes the session todo list as raw text, or JSON matching { todos } when --json is passed",
		),
		FailureOutput: new("writes storage or validation errors to stderr and exits non-zero"),
		Output:        &declare.LeafOutput{JSON: listOutput},
		Kind:          &declare.RPC[declare.NativeOperation]{},
	}, host.RPCHandlerFunc(list)), nil
}

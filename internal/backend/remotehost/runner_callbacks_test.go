package remotehost_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/remotehost/testdata/fixture"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
)

// callbackLeaf obtains command schemas from their single generated declarations.
func callbackLeaf(t *testing.T, name, summary string, schema json.RawMessage) declare.Leaf[declare.NativeOperation] {
	t.Helper()
	leaf := declare.Leaf[declare.NativeOperation]{Name: name, Summary: summary, Kind: &declare.RPC[declare.NativeOperation]{}}
	if schema != nil {
		var err error
		leaf.Input, err = declare.NewSchema(schema)
		requirePipe(t, err)
	}
	return leaf
}

// callbackSelection binds RPC declarations without native package dependencies.
func callbackSelection(t *testing.T, commands *host.CommandSet) *remotehost.CommandSelection {
	t.Helper()
	catalog, err := remotehost.NewCommandCatalog(nil, nil)
	requirePipe(t, err)
	selected, err := catalog.Select(commands)
	requirePipe(t, err)
	return selected
}

func TestRunnerDeclaredCallbacksStorageInputAndCancellation(t *testing.T) {
	started := make(chan struct{})
	stopped := make(chan struct{})
	add := callbackLeaf(t, "add", "Add a todo.", fixture.TodoArgsJSONSchema())
	add.Positionals = new([]string{"text"})
	note := callbackLeaf(t, "note", "Take a note from stdin.", fixture.TodoArgsJSONSchema())
	note.StdinField = new("text")
	hold := callbackLeaf(t, "hold", "Wait.", fixture.HoldArgsJSONSchema())
	hold.Positionals = new([]string{"ms"})
	commands := &host.CommandSet{}
	addHandler := host.TypedRPC(fixture.DecodeTodoArgs, func(ctx context.Context, call host.Call[fixture.TodoArgs], p host.RPCPort) (uint8, error) {
		items, err := host.Update(ctx, p, "todos", hosttest.DecodeItems, func(items hosttest.Items) ([]byte, error) { return items.MarshalJSON() }, func(current *hosttest.Items) (hosttest.Items, error) {
			items := hosttest.Items{}
			if current != nil {
				items = append(items, (*current)...)
			}
			return append(items, call.Args.Text), nil
		})
		if err != nil {
			return 0, err
		}
		return 0, p.Stdout(ctx, []byte(fmt.Sprintf("added %d\n", len(items))))
	})
	listHandler := host.RPCHandlerFunc(func(ctx context.Context, _ host.RPCInvocation, p host.RPCPort) (uint8, error) {
		reply, err := p.Storage(ctx, &host.StorageRead{Key: "todos"})
		if err != nil {
			return 0, err
		}
		value, ok := reply.(*host.StorageValue)
		if !ok {
			return 0, errors.New("a read answers a value")
		}
		var items hosttest.Items
		if string(value.Value) != "null" {
			items, err = hosttest.DecodeItems(value.Value)
			if err != nil {
				return 0, err
			}
		}
		return 0, p.Stdout(ctx, []byte(strings.Join(items, ",")+"\n"))
	})
	noteHandler := host.TypedRPC(fixture.DecodeTodoArgs, func(ctx context.Context, call host.Call[fixture.TodoArgs], p host.RPCPort) (uint8, error) {
		return 0, p.Stdout(ctx, []byte("noted: "+call.Args.Text))
	})
	requirePipe(t, commands.Register(host.Group("todo", "Todos.", host.Leaf(add, addHandler), host.Leaf(callbackLeaf(t, "list", "List the todos.", nil), listHandler), host.Leaf(note, noteHandler))))
	holdHandler := host.TypedRPC(fixture.DecodeHoldArgs, func(ctx context.Context, call host.Call[fixture.HoldArgs], _ host.RPCPort) (uint8, error) {
		close(started)
		timer := time.NewTimer(time.Duration(call.Args.Ms) * time.Millisecond)
		defer timer.Stop()
		select {
		case <-timer.C:
			return 0, nil
		case <-ctx.Done():
			close(stopped)
			return 130, nil
		}
	})
	spew := host.RPCHandlerFunc(func(ctx context.Context, _ host.RPCInvocation, p host.RPCPort) (uint8, error) {
		for block := range 100 {
			var lines strings.Builder
			for line := 1; line <= 1000; line++ {
				fmt.Fprintf(&lines, "line %d\n", block*1000+line)
			}
			if err := p.Stdout(ctx, []byte(lines.String())); err != nil {
				return 0, err
			}
		}
		return 0, nil
	})
	requirePipe(t, commands.Register(host.Group("probe", "Probes.", host.Leaf(hold, holdHandler), host.Leaf(callbackLeaf(t, "line", "Answer the first line typed.", nil), host.RPCHandlerFunc(firstLine)), host.Leaf(callbackLeaf(t, "spew", "Print many lines.", nil), spew))))
	f := runnerFixture(t, remotehosttest.FixtureOptions{Commands: commands})
	s, p := runnerShell(t, f, nil, callbackSelection(t, commands))
	added := runnerExec(t, s, "todo add first && todo add second && todo list", 10000)
	if added.State.ExitCode != 0 || added.Stdout.Delta != "added 1\nadded 2\nfirst,second\n" {
		t.Fatal(added)
	}
	stored, err := hosttest.DecodeItems(f.Policy().NodeStorage("test-session").Value("todos"))
	requirePipe(t, err)
	if len(stored) != 2 || stored[0] != "first" || stored[1] != "second" {
		t.Fatal(stored)
	}
	if runnerExec(t, s, "printf 'from stdin' | todo note", 10000).Stdout.Delta != "noted: from stdin" {
		t.Fatal("finite stdin missing")
	}
	script := `probe spew | head -n 1; echo "status=${PIPESTATUS[0]}"`
	headed := runnerExec(t, s, script+"; bash -c '"+script+"'", 10000)
	if headed.State.ExitCode != 0 || headed.Stdout.Delta != "line 1\nstatus=141\nline 1\nstatus=141\n" || headed.Stderr.Delta != "" {
		t.Fatal(headed)
	}
	typing := runnerExec(t, s, "probe line", 300)
	if typing.State.Phase != host.Running {
		t.Fatal(typing)
	}
	requirePipe(t, s.Write(t.Context(), typing.CommandID, []byte("typed\n")))
	typed := runnerEnd(t, s, p, typing.CommandID)
	if typed.State.ExitCode != 0 || typed.Output.Tail != "typed|test-conversation" {
		t.Fatal(typed)
	}
	usage := runnerExec(t, s, "todo add", 10000)
	if usage.State.ExitCode == 0 || usage.Stderr.Tail == "" {
		t.Fatal(usage)
	}
	holding := runnerExec(t, s, "probe hold 30000", 300)
	if holding.State.Phase != host.Running {
		t.Fatal(holding)
	}
	<-started
	requirePipe(t, s.Abort(t.Context(), holding.CommandID))
	<-stopped
	aborted, err := s.Status(holding.CommandID)
	requirePipe(t, err)
	if aborted.State.Phase != host.Aborted {
		t.Fatal(aborted)
	}
}

func TestRunnerNestedGroupHelpAndValidatedJSONOutput(t *testing.T) {
	emit := callbackLeaf(t, "emit", "Print a text.", fixture.EmitArgsJSONSchema())
	emit.Positionals = new([]string{"text"})
	schema, err := declare.NewSchema(fixture.EmittedJSONSchema())
	requirePipe(t, err)
	emit.Output = &declare.LeafOutput{JSON: schema}
	handler := host.TypedRPC(fixture.DecodeEmitArgs, func(ctx context.Context, call host.Call[fixture.EmitArgs], p host.RPCPort) (uint8, error) {
		return 0, p.Stdout(ctx, []byte(call.Args.Text))
	})
	commands := &host.CommandSet{}
	requirePipe(t, commands.Register(host.Group("probe", "Probes.", host.Group("json", "Output probes.", host.Leaf(emit, handler)))))
	f := runnerFixture(t, remotehosttest.FixtureOptions{Commands: commands})
	s, _ := runnerShell(t, f, nil, callbackSelection(t, commands))
	help := runnerExec(t, s, "probe json --help", 10000)
	if help.State.ExitCode != 0 || !strings.HasPrefix(help.Stdout.Delta, "probe json: Output probes.\n") || !strings.Contains(help.Stdout.Delta, "  probe json emit <text> [--json]\n") {
		t.Fatal(help)
	}
	raw := runnerExec(t, s, "probe json emit 'not json'", 10000)
	if raw.State.ExitCode != 0 || raw.Stdout.Delta != "not json" {
		t.Fatal(raw)
	}
	valid := runnerExec(t, s, `probe json emit '{"ok":true}' --json`, 10000)
	if valid.State.ExitCode != 0 || valid.Stdout.Delta != `{"ok":true}` {
		t.Fatal(valid)
	}
	invalid := runnerExec(t, s, "probe json emit 'not json' --json", 10000)
	mismatch := runnerExec(t, s, `probe json emit '{"ok":1}' --json`, 10000)
	for _, refused := range []host.CommandStatus{invalid, mismatch} {
		if refused.State.ExitCode != 1 || refused.Stdout.Delta != "" {
			t.Fatal(refused)
		}
	}
	if !strings.HasPrefix(invalid.Stderr.Delta, "demi-runner: --json output is not JSON: ") {
		t.Fatal(invalid.Stderr)
	}
	if mismatch.Stderr.Delta != "demi-runner: --json output does not match its schema: \"ok\" is not of type \"boolean\"\n" {
		t.Fatal(mismatch.Stderr)
	}
	usage := runnerExec(t, s, "probe json emit", 10000)
	if usage.State.ExitCode != 1 || !strings.Contains(usage.Stderr.Delta, `"text" is a required property`) {
		t.Fatal(usage)
	}
}

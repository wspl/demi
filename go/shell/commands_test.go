package shell_test

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"reflect"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/shell"
	"github.com/wspl/demi/go/shell/shelltest"
)

func todo(t *testing.T, h shell.RPCHandler) shell.Declared {
	t.Helper()
	node, err := commandtree.DecodeNode([]byte(`{"name":"add","summary":"Add a todo.","kind":"rpc","positionals":["text"],"input":{"type":"object","properties":{"text":{"type":"string","maxLength":1},"count":{"type":"integer","minimum":0}},"required":["text"],"additionalProperties":false}}`))
	if err != nil {
		t.Fatal(err)
	}
	return shell.DeclareGroup("demi", "Demi commands.", shell.DeclareGroup("todo", "Todos.", shell.DeclareLeaf(node.(commandtree.Leaf), h)))
}
func invoke(args string) shell.RPCInvocation {
	return shell.RPCInvocation{Path: []string{"demi", "todo", "add"}, Args: jsontext.Value(args), Context: shelltest.CommandContext(), Cwd: "/"}
}
func TestCommandsRefuseAtomicallyAndDispatchUncoercedArguments(t *testing.T) {
	calls := 0
	handler := shell.RPCHandlerFunc(func(ctx context.Context, call shell.RPCInvocation, p shell.RPCPort) (uint8, error) {
		calls++
		return 0, p.Stdout(ctx, []byte("called\n"))
	})
	var commands shell.CommandSet
	if err := commands.Register(todo(t, handler)); err != nil {
		t.Fatal(err)
	}
	before := commands.RenderHelp()
	if !strings.HasPrefix(before, commandtree.HelpDefaults+"\n\n") || !strings.Contains(before, "demi todo add <text>") {
		t.Fatalf("model help: %s", before)
	}
	if err := commands.Register(todo(t, handler)); err == nil || err.Error() != `command "demi" is already registered` {
		t.Fatalf("duplicate: %v", err)
	}
	for _, name := range []string{"git", "cd", "grep", "python3", "."} {
		var set shell.CommandSet
		if err := set.Register(shell.DeclareLeaf(commandtree.Leaf{Name: name, Kind: commandtree.KindRPC}, handler)); err == nil || !strings.Contains(err.Error(), "reserved for shell and system commands") {
			t.Fatalf("reserved %s: %v", name, err)
		}
	}
	if err := commands.Graft([]string{"demi"}, shell.DeclareLeaf(commandtree.Leaf{Name: "todo", Kind: commandtree.KindRPC}, nil)); err == nil || err.Error() != `rpc command "demi todo" has no handler` {
		t.Fatalf("unbound graft: %v", err)
	}
	if commands.RenderHelp() != before {
		t.Fatal("failed graft changed commands")
	}
	memory := shelltest.NewMemoryPort(nil)
	for _, args := range []string{`{"text":"a","count":"7"}`, `{"text":"a","extra":1}`, `{"text":"ab"}`} {
		_, err := commands.Dispatch(t.Context(), invoke(args), memory.Port(t.Context()))
		var rpc *shell.RPCError
		if !errors.As(err, &rpc) || rpc.Kind != shell.RPCUsage || !strings.HasPrefix(err.Error(), "Invalid command arguments") {
			t.Fatalf("invalid %s: %v", args, err)
		}
	}
	if calls != 0 {
		t.Fatal("invalid arguments reached handler")
	}
	if _, err := commands.Dispatch(t.Context(), invoke(`{"text":"𝄞","count":2}`), memory.Port(t.Context())); err != nil {
		t.Fatal(err)
	}
	if calls != 1 || string(memory.Stdout()) != "called\n" {
		t.Fatal("valid scalar text did not reach handler")
	}
	replacement := shell.DeclareLeaf(commandtree.Leaf{Name: "add", Summary: "Replacement.", Kind: commandtree.KindRPC}, handler)
	if err := commands.Graft([]string{"demi", "todo"}, replacement); err != nil {
		t.Fatal(err)
	}
	if _, err := commands.Dispatch(t.Context(), invoke(`{}`), memory.Port(t.Context())); err != nil {
		t.Fatal(err)
	}
	if calls != 2 || !strings.Contains(commands.RenderHelp(), "Replacement.") {
		t.Fatal("replacement not installed")
	}
	if got := commands.Filter(func([]string) bool { return false }); len(got.Declarations()) != 0 || got.RenderHelp() != "" {
		t.Fatal("empty groups survived filtering")
	}
}

// firstReads is a test transport that ensures both updates have read the same
// revision before either writes, so the scenario necessarily exercises retry.
type firstReads struct {
	memory  *shelltest.MemoryPort
	arrived *sync.WaitGroup
	reads   atomic.Int32
}

func (p *firstReads) Request(ctx context.Context, request shell.PortRequest) (shell.PortResponse, error) {
	response, err := p.memory.Request(ctx, request)
	if storage, ok := request.(shell.PortStorageRequest); ok {
		if _, read := storage.Op.(shell.StorageRead); read && p.reads.Add(1) == 1 {
			p.arrived.Done()
			p.arrived.Wait()
		}
	}
	return response, err
}
func TestStorageUpdatesRetryConflictsAndPreserveUnreadableValues(t *testing.T) {
	storage := &shelltest.MemoryStorage{}
	var first sync.WaitGroup
	first.Add(2)
	errs := make(chan error, 2)
	decode := func(data []byte) ([]string, error) {
		var values []string
		err := json.Unmarshal(data, &values)
		return values, err
	}
	encode := func(values []string) ([]byte, error) { return json.Marshal(values) }
	for _, item := range []string{"A", "B"} {
		memory := shelltest.NewMemoryPort(storage)
		port := shell.RPCPort{Context: t.Context(), Transport: &firstReads{memory: memory, arrived: &first}}
		go func() {
			_, err := shell.Update(t.Context(), port, "todos", decode, encode, func(current *[]string) ([]string, error) {
				var values []string
				if current != nil {
					values = *current
				}
				return append(values, item), nil
			})
			errs <- err
		}()
	}
	for range 2 {
		if err := <-errs; err != nil {
			t.Fatal(err)
		}
	}
	values, err := decode(storage.Value("todos"))
	if err != nil {
		t.Fatal(err)
	}
	slices.Sort(values)
	if !reflect.DeepEqual(values, []string{"A", "B"}) {
		t.Fatalf("lost a competing write: %v", values)
	}
	corrupt := jsontext.Value(`{"not":"a list"}`)
	storage.Apply(shell.StorageWriteIf{Key: "todos", Value: corrupt})
	called := false
	_, err = shell.Update(t.Context(), shelltest.NewMemoryPort(storage).Port(t.Context()), "todos", decode, encode, func(current *[]string) ([]string, error) {
		called = true
		return nil, nil
	})
	if err == nil || !strings.HasPrefix(err.Error(), "stored todos is unreadable:") || called || string(storage.Value("todos")) != string(corrupt) {
		t.Fatalf("corrupt storage changed: %v", err)
	}
}

package host_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
)

// Command scenarios are in memory and cost less than one second; no external programs run.
func schema(t *testing.T, data []byte) *declare.Schema {
	t.Helper()
	s, err := declare.NewSchema(data)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func rpcLeaf(name string) declare.Leaf[declare.NativeOperation] {
	return declare.Leaf[declare.NativeOperation]{
		Name:    name,
		Summary: "Add.",
		Kind:    &declare.RPC[declare.NativeOperation]{},
	}
}

func addHandler() host.RPCHandler {
	return host.TypedRPC(
		hosttest.DecodeAddArgs,
		func(ctx context.Context, call host.Call[hosttest.AddArgs], port host.RPCPort) (uint8, error) {
			count := uint32(1)
			if call.Args.Count != nil {
				count = *call.Args.Count
			}
			added, err := host.Update(
				ctx,
				port,
				"todos",
				hosttest.DecodeItems,
				func(v hosttest.Items) ([]byte, error) { return v.MarshalJSON() },
				func(current hosttest.Items, found bool) (hosttest.Items, error) {
					items := hosttest.Items{}
					if found {
						items = append(items, current...)
					}
					for range count {
						items = append(items, call.Args.Text)
					}
					return items, nil
				},
			)
			if err != nil {
				return 0, err
			}
			return 0, port.Stdout(ctx, []byte(fmt.Sprintf("%d\n", len(added))))
		},
	)
}

func todo(t *testing.T) host.Declared {
	t.Helper()
	add := rpcLeaf("add")
	add.Summary = "Add a new todo."
	add.Input = schema(t, hosttest.AddArgsJSONSchema())
	positionals := []string{"text"}
	add.Positionals = &positionals
	add.Output = &declare.LeafOutput{JSON: schema(t, hosttest.ReplyJSONSchema())}
	read := declare.Leaf[declare.NativeOperation]{
		Name:    "read",
		Summary: "Read a file.",
		Input:   add.Input,
		Kind: &declare.Native[declare.NativeOperation]{
			Binding: declare.NativeOperation{Package: "demi.file", Operation: "file.read"},
		},
	}
	return host.Group(
		"demi",
		"Demi commands.",
		host.Group("todo", "Manage the todo list.", host.Leaf(add, addHandler()), host.Leaf(read, nil)),
	)
}

func invocation(path []string, args string) host.RPCInvocation {
	// Test arguments are decoded through the same generated boundary as outside invocations.
	raw := `{"path":` + quotePath(
		path,
	) + `,"argv":[],"args":` + args + `,"json":false,"cwd":"/","env":{},"context":{"conversation":"test-conversation",` +
		`"caller":{"kind":"agent","number":1},"locale":{"timeZone":"Asia/Shanghai",` +
		`"languages":["zh-CN","en"]}},"stdin":false}`
	v, err := host.DecodeRPCInvocation([]byte(raw))
	if err != nil {
		panic(err)
	} // Only fixed test fixture construction; never production input.
	return v
}

func quotePath(path []string) string {
	data, err := contract.EncodeJSON(path)
	if err != nil {
		panic(err)
	}
	return string(data)
}

func TestRegistrationRefusesReservedTakenMalformedAndUnbound(t *testing.T) {
	var set host.CommandSet
	if err := set.Register(todo(t)); err != nil {
		t.Fatal(err)
	}
	if err := set.Register(todo(t)); err == nil || !strings.Contains(err.Error(), "already registered") {
		t.Fatalf("duplicate: %v", err)
	}
	for _, name := range []string{"git", "cd", "grep", "python3", "."} {
		if !slices.Contains(host.ReservedNames(), name) {
			t.Fatalf("not reserved: %s", name)
		}
		var empty host.CommandSet
		if err := empty.Register(
			host.Leaf(rpcLeaf(name), nil),
		); err == nil ||
			!strings.Contains(err.Error(), "reserved") {
			t.Fatalf("reserved %s: %v", name, err)
		}
	}
	args := schema(t, hosttest.AddArgsJSONSchema())
	text, count, missing := []string{"text"}, []string{"count", "text"}, []string{"missing"}
	fieldCount, fieldText := "count", "text"
	base := rpcLeaf("add")
	base.Input = args
	with := func(f func(*declare.Leaf[declare.NativeOperation])) host.Declared {
		leaf := base
		f(&leaf)
		return host.Leaf(leaf, addHandler())
	}
	tests := []struct {
		d    host.Declared
		want string
	}{
		{host.Leaf(rpcLeaf("add"), nil), `"demi add" has no handler`},
		{host.Group("empty", "Empty."), "no subcommands"},
		{host.Leaf(rpcLeaf("bad name"), addHandler()), "invalid command name"},
		{with(func(l *declare.Leaf[declare.NativeOperation]) { l.Positionals = &missing }), "missing"},
		{
			with(func(l *declare.Leaf[declare.NativeOperation]) { l.StdinField = &fieldCount }),
			"stdin input must be a string",
		},
		{
			with(func(l *declare.Leaf[declare.NativeOperation]) {
				l.Positionals = &text
				l.StdinField = &fieldText
			}),
			"multiple input sources for text",
		},
		{
			with(func(l *declare.Leaf[declare.NativeOperation]) { l.Positionals = &count }),
			"required positional follows optional positional",
		},
		{with(func(l *declare.Leaf[declare.NativeOperation]) {
			l.Kind = &declare.Native[declare.NativeOperation]{
				Binding: declare.NativeOperation{Package: "demi.file", Operation: "file.read"},
			}
		}), "takes no handler"},
		{host.Leaf(base, addHandler()).Describe("absent", "x"), "absent"},
		{host.Leaf(rpcLeaf("add"), addHandler()).Describe("text", "x"), "before its input"},
	}
	for _, tc := range tests {
		t.Run(tc.want, func(t *testing.T) {
			var s host.CommandSet
			err := s.Register(host.Group("demi", "Demi.", tc.d))
			if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("registration: %v", err)
			}
			if len(s.Declarations()) != 0 {
				t.Fatal("refused root was installed")
			}
		})
	}
}

func TestRegistrationRefusesInputSubsetNamingField(t *testing.T) {
	for _, tc := range []struct{ field, property, reason string }{
		{
			"count",
			`{"type":"integer","default":2}`,
			"default",
		},
		{
			"inner",
			`{"type":"object","properties":{"text":{"type":"string"}}}`,
			"nested object",
		},
	} {
		t.Run(tc.field, func(t *testing.T) {
			leaf := rpcLeaf("add")
			leaf.Input = schema(
				t,
				[]byte(`{"type":"object","properties":{"`+tc.field+`":`+tc.property+`},"additionalProperties":false}`),
			)
			var s host.CommandSet
			err := s.Register(host.Group("demi", "Demi.", host.Leaf(leaf, addHandler())))
			if err == nil {
				t.Fatal("invalid input accepted")
			}
			for _, want := range []string{`"demi add"`, `input "` + tc.field + `"`, tc.reason} {
				if !strings.Contains(err.Error(), want) {
					t.Fatalf("%v lacks %q", err, want)
				}
			}
		})
	}
}

func TestHelpDefaultsAndEveryRoot(t *testing.T) {
	var s host.CommandSet
	if s.RenderHelp() != "" {
		t.Fatal("empty help")
	}
	if err := s.Register(todo(t)); err != nil {
		t.Fatal(err)
	}
	root := s.Declarations()[0].Help("demi")
	if s.RenderHelp() != declare.HelpDefaults+"\n\n"+root ||
		!strings.Contains(root, "demi todo add <text> [--count <count>] [--json]") ||
		!strings.Contains(root, "How many copies") {
		t.Fatal(s.RenderHelp())
	}
}

func TestDispatchValidatesWireArgumentsAndRunsHandler(t *testing.T) {
	var s host.CommandSet
	if err := s.Register(todo(t)); err != nil {
		t.Fatal(err)
	}
	memory := hosttest.NewMemoryPort(nil)
	call := func(args string) (uint8, error) {
		return s.Dispatch(t.Context(), invocation([]string{"demi", "todo", "add"}, args), memory.Port())
	}
	_, err := call(`{"text":"a","count":"7"}`)
	var rpc *host.RPCError
	if !errors.As(err, &rpc) || rpc.Kind != host.Usage || !strings.HasPrefix(err.Error(), "Invalid command arguments") {
		t.Fatalf("wrong numeric type: %v", err)
	}
	if _, err = call(`{"text":"a","extra":1}`); err == nil {
		t.Fatal("unknown field accepted")
	}
	if code, err := call(`{"text":"𝄞","count":2}`); err != nil || code != 0 {
		t.Fatalf("unicode scalar: %d %v", code, err)
	}
	if _, err = call(`{"text":"ab"}`); err == nil {
		t.Fatal("long text accepted")
	}
	if string(memory.Stdout()) != "2\n" {
		t.Fatalf("output %q", memory.Stdout())
	}
	_, err = s.Dispatch(t.Context(), invocation([]string{"demi", "todo", "read"}, `{"text":"a"}`), memory.Port())
	if !errors.As(err, &rpc) || rpc.Kind != host.Usage || !strings.Contains(err.Error(), "not an rpc command") {
		t.Fatalf("native dispatch: %v", err)
	}
}

func TestConcurrentUpdatesKeepBothWrites(t *testing.T) {
	storage := &hosttest.MemoryStorage{}
	// Force both handlers to read revision zero before either writes: this proves the conflict retry.
	var ready sync.WaitGroup
	ready.Add(2)
	var workers sync.WaitGroup
	errs := make(chan error, 2)
	for _, item := range []string{"A", "B"} {
		workers.Go(func() {
			first := true
			defer func() {
				if first {
					ready.Done()
				}
			}()
			_, err := host.Update(
				t.Context(),
				hosttest.NewMemoryPort(storage).Port(),
				"todos",
				hosttest.DecodeItems,
				func(v hosttest.Items) ([]byte, error) { return v.MarshalJSON() },
				func(current hosttest.Items, found bool) (hosttest.Items, error) {
					if first {
						first = false
						ready.Done()
						ready.Wait()
					}
					items := hosttest.Items{}
					if found {
						items = append(items, current...)
					}
					return append(items, item), nil
				},
			)
			errs <- err
		})
	}
	workers.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	items, err := hosttest.DecodeItems(storage.Value("todos"))
	if err != nil {
		t.Fatal(err)
	}
	slices.Sort(items)
	if !reflect.DeepEqual(items, hosttest.Items{"A", "B"}) {
		t.Fatalf("stored %v", items)
	}
}

func TestUnreadableStoredValueIsNotReplaced(t *testing.T) {
	storage := &hosttest.MemoryStorage{}
	storage.Apply(&host.StorageWriteIf{Key: "todos", Value: json.RawMessage(`{"not":"a list"}`)})
	_, err := host.Update(
		t.Context(),
		hosttest.NewMemoryPort(storage).Port(),
		"todos",
		hosttest.DecodeItems,
		func(v hosttest.Items) ([]byte, error) { return v.MarshalJSON() },
		func(_ hosttest.Items, _ bool) (hosttest.Items, error) {
			t.Fatal("change called for unreadable data")
			return nil, nil
		},
	)
	var rpc *host.RPCError
	if !errors.As(err, &rpc) || rpc.Kind != host.HandlerFailed || !strings.Contains(err.Error(), "unreadable") {
		t.Fatalf("unreadable: %v", err)
	}
	if string(storage.Value("todos")) != `{"not":"a list"}` {
		t.Fatal("stored value changed")
	}
}

func TestGraftAndFilterKeepBindingsAtomic(t *testing.T) {
	var s host.CommandSet
	if err := s.Register(todo(t)); err != nil {
		t.Fatal(err)
	}
	calls := 0
	handler := host.RPCHandlerFunc(
		func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) {
			calls++
			return 0, nil
		},
	)
	agent := host.Group("agent", "Agents.", host.Leaf(rpcLeaf("list"), handler))
	if err := s.Graft([]string{"demi"}, agent); err != nil {
		t.Fatal(err)
	}
	names := func(s *host.CommandSet) []string {
		var result []string
		for _, n := range s.Declarations()[0].(*declare.Group[declare.NativeOperation]).Subcommands {
			result = append(result, declare.Name(n))
		}
		return result
	}
	if !reflect.DeepEqual(names(&s), []string{"todo", "agent"}) {
		t.Fatal(names(&s))
	}
	invoke := func() {
		t.Helper()
		if _, err := s.Dispatch(
			t.Context(),
			invocation([]string{"demi", "agent", "list"}, `{}`),
			hosttest.NewMemoryPort(nil).Port(),
		); err != nil {
			t.Fatal(err)
		}
	}
	invoke()
	if calls != 1 {
		t.Fatal(calls)
	}
	if err := s.Graft([]string{"demi"}, host.Leaf(rpcLeaf("agent"), nil)); err == nil {
		t.Fatal("unbound graft accepted")
	}
	invoke()
	if err := s.Graft([]string{"demi", "todo", "add"}, host.Leaf(rpcLeaf("x"), nil)); err == nil {
		t.Fatal("grafted below leaf")
	}
	narrow := s.Filter(func(path []string) bool { return len(path) < 2 || path[1] != "todo" })
	if !reflect.DeepEqual(names(narrow), []string{"agent"}) {
		t.Fatal(names(narrow))
	}
	if len(s.Filter(func([]string) bool { return false }).Declarations()) != 0 {
		t.Fatal("empty groups remain")
	}
	// Successful replacement must remove old handlers as well as replace the declaration.
	if err := s.Graft(
		[]string{"demi"},
		host.Group("agent", "Agents.", host.Leaf(rpcLeaf("new"), handler)),
	); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Check(invocation([]string{"demi", "agent", "list"}, `{}`)); err == nil {
		t.Fatal("old handler retained")
	}
}

func TestDescriptionReplacementAndServedDeclarations(t *testing.T) {
	leaf := rpcLeaf("add")
	leaf.Input = schema(t, hosttest.AddArgsJSONSchema())
	declared := host.Leaf(leaf, addHandler()).Describe("count", "How many copies; default 2.")
	var s host.CommandSet
	if err := s.Register(declared); err != nil {
		t.Fatal(err)
	}
	updated := declare.AsLeaf(s.Declarations()[0])
	if !strings.Contains(string(updated.Input.Document()), "How many copies; default 2.") ||
		strings.Contains(string(leaf.Input.Document()), "default 2") {
		t.Fatal("description failed or mutated source")
	}
	var served host.CommandSet
	if err := served.Register(host.Served(s.Declarations()[0], addHandler())); err != nil {
		t.Fatal(err)
	}
	memory := hosttest.NewMemoryPort(nil)
	if _, err := served.Dispatch(t.Context(), invocation([]string{"add"}, `{"text":"x"}`), memory.Port()); err != nil {
		t.Fatal(err)
	}
	if string(memory.Stdout()) != "1\n" {
		t.Fatal("served leaf did not run")
	}
}

func TestDescriptionKeepsPropertyOrderAndFirstRefusal(t *testing.T) {
	leaf := rpcLeaf("input")
	leaf.Input = schema(
		t,
		[]byte(
			`{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}},"additionalProperties":false}`,
		),
	)
	var set host.CommandSet
	if err := set.Register(host.Leaf(leaf, addHandler()).Describe("z", "Dynamic")); err != nil {
		t.Fatal(err)
	}
	doc := string(declare.AsLeaf(set.Declarations()[0]).Input.Document())
	if strings.Index(doc, `"z"`) > strings.Index(doc, `"a"`) {
		t.Fatalf("properties reordered: %s", doc)
	}
	var refused host.CommandSet
	err := refused.Register(host.Leaf(leaf, addHandler()).Describe("first", "x").Describe("second", "y"))
	if err == nil || !strings.Contains(err.Error(), "first") {
		t.Fatalf("first refusal lost: %v", err)
	}
	var unbound host.CommandSet
	err = unbound.Register(host.Group("root", "Root.", host.Leaf(rpcLeaf("z"), nil), host.Leaf(rpcLeaf("a"), nil)))
	if err == nil || !strings.Contains(err.Error(), `"root z"`) {
		t.Fatalf("first unbound leaf: %v", err)
	}
}

package todo

import (
	"bytes"
	"context"
	"errors"
	"os"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type capturedPort interface {
	host.PortTransport
	Stdout() []byte
}

// callTodo sends a shell command through the same JSON boundary as the plugin host.
func callTodo(ctx context.Context, f *Factory, storage *hosttest.MemoryStorage, transport capturedPort, args ...string) (string, error) {
	roots, err := plugintest.Roots(f.Manifest())
	if err != nil {
		return "", err
	}
	parsed, err := plugintest.Parse(roots[0], args, nil)
	if err != nil {
		return "", err
	}
	if transport == nil {
		transport = hosttest.NewMemoryPort(storage)
	}
	values, err := parsed.Values.MarshalJSON()
	if err != nil {
		return "", err
	}
	invocation := host.RPCInvocation{Path: parsed.Path[1:], Argv: args, Args: values, JSON: parsed.JSON, CWD: "/workspace", Env: map[string]string{}, Context: hosttest.CommandContext()}
	reply, err := plugintest.Loopback(f.Instance()).Call(ctx, &plugin.RequestCommand{User: "u1", Invocation: invocation}, plugintest.Port(transport))
	if err != nil {
		return "", err
	}
	if exit, ok := reply.(*plugin.ReplyExit); !ok || exit.Code != 0 {
		return "", &host.RPCError{Kind: host.HandlerFailed, Message: "unexpected plugin reply"}
	}
	return string(transport.Stdout()), nil
}

// No subprocesses or waits; the complete command scenario costs below one second.
func TestTodosAreAddedListedChangedAndDoneAsLinesOrJSON(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	storage := &hosttest.MemoryStorage{}
	steps := []struct {
		args []string
		want string
	}{
		{[]string{"todo", "list"}, "No todos.\n"},
		{[]string{"todo", "add", "Run tests"}, "[ ] T1 Run tests\n"},
		{[]string{"todo", "add", "Write docs", "--json"}, `{"todo":{"id":"T2","text":"Write docs","status":"pending"}}`},
		{[]string{"todo", "list"}, "[ ] T1 Run tests\n[ ] T2 Write docs\n"},
		{[]string{"todo", "update", "T1", "--text", "Run full tests"}, "[ ] T1 Run full tests\n"},
		{[]string{"todo", "update", "T1", "--status", "in_progress", "--json"}, `{"todo":{"id":"T1","text":"Run full tests","status":"in_progress"}}`},
		{[]string{"todo", "list"}, "[-] T1 Run full tests\n[ ] T2 Write docs\n"},
		{[]string{"todo", "done", "T2"}, "[x] T2 Write docs\n"},
		{[]string{"todo", "done", "T1", "--json"}, `{"todo":{"id":"T1","text":"Run full tests","status":"done"}}`},
		{[]string{"todo", "list", "--json"}, `{"todos":[{"id":"T1","text":"Run full tests","status":"done"},{"id":"T2","text":"Write docs","status":"done"}]}`},
	}
	for _, step := range steps {
		got, err := callTodo(t.Context(), f, storage, nil, step.args...)
		if err != nil {
			t.Fatal(err)
		}
		if got != step.want {
			t.Fatalf("%v: got %q, want %q", step.args, got, step.want)
		}
	}
	before := storage.Value(storageKey)
	_, err = callTodo(t.Context(), f, storage, nil, "todo", "done", "T9")
	var failed *plugin.ErrorFailed
	if !errors.As(err, &failed) || failed.Message != "Todo not found: T9" {
		t.Fatalf("missing todo: %v", err)
	}
	if !bytes.Equal(before, storage.Value(storageKey)) {
		t.Fatal("failed command changed storage")
	}
}

// firstReads forces both commands to read the same revision before either writes.
type firstReads struct {
	*hosttest.MemoryPort
	barrier *readBarrier
}

type readBarrier struct {
	mu    sync.Mutex
	reads int
	ready chan struct{}
}

func (p *firstReads) Request(ctx context.Context, request host.PortRequest) (host.PortResponse, error) {
	response, err := p.MemoryPort.Request(ctx, request)
	if err != nil {
		return nil, err
	}
	if stored, ok := request.(*host.PortStorage); ok {
		if _, read := stored.Op.(*host.StorageRead); read {
			p.barrier.mu.Lock()
			p.barrier.reads++
			n := p.barrier.reads
			if n == 2 {
				close(p.barrier.ready)
			}
			p.barrier.mu.Unlock()
			if n <= 2 {
				select {
				case <-ctx.Done():
					return nil, ctx.Err()
				case <-p.barrier.ready:
				}
			}
		}
	}
	return response, nil
}

// Controlled read interleaving proves CAS retries; no wall-clock waits, below one second.
func TestConcurrentAddsKeepEveryTodoWithItsOwnID(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	storage := &hosttest.MemoryStorage{}
	barrier := &readBarrier{ready: make(chan struct{})}
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	outputs := make(chan string, 2)
	var wg sync.WaitGroup
	errs := make(chan error, 2)
	for _, text := range []string{"first", "second"} {
		wg.Go(func() {
			port := &firstReads{MemoryPort: hosttest.NewMemoryPort(storage), barrier: barrier}
			output, err := callTodo(ctx, f, storage, port, "todo", "add", text, "--json")
			outputs <- output
			errs <- err
		})
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	close(outputs)
	ids := []string{}
	for output := range outputs {
		result, err := DecodeOneTodo([]byte(output))
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, result.Todo.ID)
	}
	sort.Strings(ids)
	if len(ids) != 2 || ids[0] != "T1" || ids[1] != "T2" {
		t.Fatalf("returned ids: %v", ids)
	}
	listed, err := callTodo(ctx, f, storage, nil, "todo", "list", "--json")
	if err != nil {
		t.Fatal(err)
	}
	list, err := DecodeTodoList([]byte(listed))
	if err != nil {
		t.Fatal(err)
	}
	todos := list.Todos
	if len(todos) != 2 || todos[0].ID != "T1" || todos[1].ID != "T2" {
		t.Fatalf("todos: %+v", todos)
	}
	texts := []string{todos[0].Text, todos[1].Text}
	sort.Strings(texts)
	if texts[0] != "first" || texts[1] != "second" {
		t.Fatalf("lost todos: %+v", todos)
	}
}

func TestRustManifest(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	got, err := f.Manifest().MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, bytes.TrimSpace(want)) {
		t.Fatalf("manifest differs from Rust:\ngot %s\nwant %s", got, want)
	}
}

// Corrupt stored lists must neither be printed nor overwritten; below one second.
func TestRejectsCorruptStorage(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	for _, raw := range []string{
		`{}`, `[{}]`,
		`[{"id":"T1","text":"task","status":"unknown"}]`,
		`[{"id":"T1","text":"task","status":"pending","extra":true}]`,
	} {
		t.Run(raw, func(t *testing.T) {
			storage := &hosttest.MemoryStorage{}
			storage.Apply(&host.StorageWriteIf{Key: storageKey, Value: []byte(raw)})
			for _, args := range [][]string{{"todo", "list"}, {"todo", "add", "new"}, {"todo", "update", "T1", "--text", "new"}, {"todo", "done", "T1"}} {
				_, err := callTodo(t.Context(), f, storage, nil, args...)
				var failed *plugin.ErrorFailed
				if !errors.As(err, &failed) || !strings.HasPrefix(failed.Message, "stored todos.json is unreadable:") {
					t.Fatalf("%v: %v", args, err)
				}
				if string(storage.Value(storageKey)) != raw {
					t.Fatal("corrupt storage was overwritten")
				}
			}
		})
	}
}

// Restored lists allocate after the largest parseable id and preserve JSON text.
func TestRestoredIDsAndJSONText(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	storage := &hosttest.MemoryStorage{}
	storage.Apply(&host.StorageWriteIf{Key: storageKey, Value: []byte(`[{"id":"T+7","text":"a","status":"done"},{"id":"Tbogus","text":"b","status":"pending"},{"id":"T2","text":"c","status":"pending"}]`)})
	got, err := callTodo(t.Context(), f, storage, nil, "todo", "add", "<>&\u2028\u2029", "--json")
	if err != nil {
		t.Fatal(err)
	}
	if want := "{\"todo\":{\"id\":\"T8\",\"text\":\"<>&\u2028\u2029\",\"status\":\"pending\"}}"; got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

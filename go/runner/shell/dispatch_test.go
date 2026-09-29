package shell_test

import (
	"bytes"
	"context"
	"encoding/json/v2"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/runner/shell"
)

type service struct {
	call func(*commandservice.Call) (commandservice.Completion, error)
}

func (s service) Operations() []string                                             { return []string{"run"} }
func (s service) Invoke(c *commandservice.Call) (commandservice.Completion, error) { return s.call(c) }

type services struct {
	client *commandservice.Client
	leases atomic.Int32
}

func (s *services) Acquire(_ context.Context, binding commandtree.Binding) (*commandservice.Client, func(), error) {
	if binding.DescriptorHash != "pinned" || binding.Operation != "run" {
		return nil, nil, fmt.Errorf("wrong binding: %+v", binding)
	}
	s.leases.Add(1)
	return s.client, func() { s.leases.Add(-1) }, nil
}
func commands(t *testing.T, declaration string) *shell.Commands {
	t.Helper()
	root, err := commandtree.DecodeNode([]byte(declaration))
	if err != nil {
		t.Fatal(err)
	}
	if err := commandtree.Validate(root); err != nil {
		t.Fatal(err)
	}
	return &shell.Commands{Roots: map[string]commandtree.Node{"demi": root}, Context: commandservice.CommandContext{
		Conversation: "conversation", Caller: commandservice.AgentCaller{Number: 3}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}},
	}}
}

const declaration = `{"name":"demi","summary":"Test","kind":"native","binding":{"package":"test","operation":"run","descriptorHash":"pinned"},"input":{"type":"object","properties":{"body":{"type":"string"}},"required":["body"],"additionalProperties":false},"stdinField":"body","runningHint":"working","output":{"json":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}}}`

func TestDeclaredHelpBodiesContextAndJSON(t *testing.T) {
	opts := options(t)
	opts.Commands = commands(t, declaration)
	var calls atomic.Int32
	var hints atomic.Int32
	opts.Commands.Hints = func(context.Context, string) (func(), error) {
		hints.Add(1)
		return func() { hints.Add(-1) }, nil
	}
	server := servicetest.Start(t, service{func(call *commandservice.Call) (commandservice.Completion, error) {
		calls.Add(1)
		if call.Invocation.Context.Conversation != "conversation" || call.Invocation.Cwd != opts.Dir {
			return commandservice.Completion{}, fmt.Errorf("incorrect invocation: %+v", call.Invocation)
		}
		var args struct {
			Body string `json:"body"`
		}
		if err := json.Unmarshal(call.Invocation.Args, &args); err != nil {
			return commandservice.Completion{}, err
		}
		if *call.Invocation.JSON {
			_, err := io.WriteString(call.Stdout, args.Body)
			return commandservice.Completion{}, err
		}
		_, err := io.WriteString(call.Stdout, "body="+args.Body)
		return commandservice.Completion{ExitCode: 7}, err
	}})
	resolver := &services{client: server.Client}
	opts.Commands.Services = resolver
	// No service is invoked, and live input is not read for help or parse errors.
	helpOptions := opts
	idle, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	helpOptions.Stdin = idle
	result, out := execute(t, helpOptions, "demi --help")
	if *result.Code != 0 || !strings.Contains(out.stdout.String(), "demi") || calls.Load() != 0 {
		t.Fatalf("%+v %q calls=%d", result, out.stdout.String(), calls.Load())
	}
	result, out = execute(t, opts, "demi --body invalid")
	if *result.Code != 1 || !strings.Contains(out.stderr.String(), "only from stdin") || calls.Load() != 0 {
		t.Fatalf("%+v %q", result, out.stderr.String())
	}
	for _, body := range []struct {
		data    []byte
		message string
	}{
		{[]byte{0xff}, "not UTF-8"},
		{bytes.Repeat([]byte("x"), 1024*1024+1), "exceeds 1 MiB"},
	} {
		if err := os.WriteFile(filepath.Join(opts.Dir, "body"), body.data, 0600); err != nil {
			t.Fatal(err)
		}
		result, out := execute(t, opts, "demi < body")
		if *result.Code != 1 || !strings.Contains(out.stderr.String(), body.message) || calls.Load() != 0 {
			t.Fatalf("body boundary: %+v %q calls=%d", result, out.stderr.String(), calls.Load())
		}
	}
	if err := os.WriteFile(filepath.Join(opts.Dir, "body"), bytes.Repeat([]byte("x"), 1024*1024), 0600); err != nil {
		t.Fatal(err)
	}
	// The dispatcher permits 1 MiB. Native invocation metadata separately has
	// the SDK's 256 KiB wire bound, so exercise this boundary through RPC.
	boundary := opts
	boundary.Commands = commands(t, `{"name":"demi","summary":"Test","kind":"rpc","input":{"type":"object","properties":{"body":{"type":"string"}},"required":["body"],"additionalProperties":false},"stdinField":"body"}`)
	boundary.Commands.RPC = rpc{func(_ context.Context, call shell.RPCCall, _ io.Reader, out, _ io.Writer) (uint8, error) {
		var args map[string]string
		if err := json.Unmarshal(call.Invocation.Args, &args); err != nil {
			return 1, err
		}
		if len(args["body"]) != 1024*1024 {
			return 1, fmt.Errorf("body was truncated")
		}
		_, err := io.WriteString(out, "accepted")
		return 7, err
	}}
	result, out = execute(t, boundary, "demi < body")
	if *result.Code != 7 || out.stdout.String() != "accepted" {
		t.Fatalf("body limit: %+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
	opts.Env["DEMI_CONVERSATION"] = "forged"
	result, out = execute(t, opts, "demi <<'EOF'\nhello\nEOF")
	if *result.Code != 7 || out.stdout.String() != "body=hello\n" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
	for _, tc := range []struct {
		body    string
		valid   bool
		message string
	}{
		{`{"ok":true}`, true, ""}, {`{"ok":"yes"}`, false, "does not match its schema"}, {`invalid`, false, "is not JSON"},
	} {
		result, out := execute(t, opts, "demi --json <<'EOF'\n"+tc.body+"\nEOF")
		if tc.valid {
			if *result.Code != 0 || out.stdout.String() != tc.body+"\n" {
				t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
			}
		} else if *result.Code != 1 || out.stdout.Len() != 0 || !strings.Contains(out.stderr.String(), tc.message) {
			t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
		}
	}
	if hints.Load() != 0 || resolver.leases.Load() != 0 {
		t.Fatalf("unreleased hint/service: %d %d", hints.Load(), resolver.leases.Load())
	}
}

type rpc struct {
	call func(context.Context, shell.RPCCall, io.Reader, io.Writer, io.Writer) (uint8, error)
}

func (r rpc) Invoke(ctx context.Context, c shell.RPCCall, in io.Reader, out, err io.Writer) (uint8, error) {
	return r.call(ctx, c, in, out, err)
}
func TestRPCCancellationReleasesHintAndRevokesContext(t *testing.T) {
	opts := options(t)
	opts.Commands = commands(t, `{"name":"demi","summary":"Test","kind":"rpc","runningHint":"working"}`)
	lifetime, revoke := context.WithCancel(t.Context())
	defer revoke()
	opts.Commands.Lifetime = lifetime
	entered := make(chan struct{})
	var hints atomic.Int32
	opts.Commands.Hints = func(context.Context, string) (func(), error) {
		hints.Add(1)
		return func() { hints.Add(-1) }, nil
	}
	opts.Commands.RPC = rpc{func(ctx context.Context, call shell.RPCCall, _ io.Reader, _ io.Writer, _ io.Writer) (uint8, error) {
		if len(call.Argv) != 1 || call.Argv[0] != "demi" {
			return 0, fmt.Errorf("bad argv %v", call.Argv)
		}
		close(entered)
		<-ctx.Done()
		return 0, ctx.Err()
	}}
	ctx, stop := context.WithTimeout(t.Context(), 5*time.Second)
	defer stop()
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(ctx, "demi", opts) }()
	select {
	case <-entered:
	case <-ctx.Done():
		t.Fatal("RPC did not start:", ctx.Err())
	}
	revoke()
	var result shell.Result
	select {
	case result = <-done:
	case <-ctx.Done():
		t.Fatal("RPC did not stop:", ctx.Err())
	}
	if result.Code == nil || *result.Code != 130 || hints.Load() != 0 {
		t.Fatalf("%+v hints=%d", result, hints.Load())
	}
	result, _ = execute(t, opts, "demi")
	if *result.Code != 130 {
		t.Fatal(result)
	}
}

func TestServiceCancellationClosesInputAndReleasesLease(t *testing.T) {
	opts := options(t)
	opts.Commands = commands(t, `{"name":"demi","summary":"Test","kind":"native","binding":{"package":"test","operation":"run","descriptorHash":"pinned"}}`)
	entered := make(chan struct{})
	server := servicetest.Start(t, service{func(call *commandservice.Call) (commandservice.Completion, error) {
		close(entered)
		_, err := call.Stdin.Next()
		return commandservice.Completion{}, err
	}})
	resolver := &services{client: server.Client}
	opts.Commands.Services = resolver
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	opts.Stdin = reader
	guard, stop := context.WithTimeout(t.Context(), 5*time.Second)
	defer stop()
	ctx, cancel := context.WithCancel(guard)
	defer cancel()
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(ctx, "demi", opts) }()
	select {
	case <-entered:
	case <-ctx.Done():
		t.Fatal("service did not start:", ctx.Err())
	}
	cancel()
	var result shell.Result
	select {
	case result = <-done:
	case <-guard.Done():
		t.Fatal("service did not stop")
	}
	if result.Signal != "SIGKILL" || resolver.leases.Load() != 0 {
		t.Fatalf("%+v leases=%d", result, resolver.leases.Load())
	}
}

func TestHelpAndNonReadingServiceLeaveLiveInputForNextCommand(t *testing.T) {
	opts := options(t)
	opts.Live = true
	opts.Commands = commands(t, `{"name":"demi","summary":"Test","kind":"native","binding":{"package":"test","operation":"run","descriptorHash":"pinned"}}`)
	var calls atomic.Int32
	server := servicetest.Start(t, service{func(*commandservice.Call) (commandservice.Completion, error) {
		calls.Add(1)
		return commandservice.Completion{}, nil
	}})
	opts.Commands.Services = &services{client: server.Client}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	opts.Stdin = reader
	if _, err := io.WriteString(writer, "retained\n"); err != nil {
		t.Fatal(err)
	}
	result, out := execute(t, opts, `demi --help >/dev/null; demi; read value; printf '%s' "$value"`)
	if *result.Code != 0 || out.stdout.String() != "retained" || calls.Load() != 1 {
		t.Fatalf("%+v %q calls=%d", result, out.stdout.String(), calls.Load())
	}
}

package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"os"
	"os/exec"
	"slices"
	"sync"
	"testing"
	"time"

	cs "github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

func invocation(op string) cs.Invocation {
	return cs.Invocation{
		Operation:    op,
		InvocationID: "test",
		Context: cs.CommandContext{
			Conversation: "c1",
			Caller:       cs.CommandCaller{Kind: "user"},
			Locale:       cs.CommandLocale{TimeZone: "UTC", Languages: []string{"en-US"}},
		},
		Args: []byte(`{}`),
		Cwd:  "/tmp",
		Env:  map[string]string{},
	}
}

// clientFor owns both ends of an in-process command connection and joins Serve.
func clientFor(t *testing.T, h cs.Handler) (*cs.Client, context.Context) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	a, b := net.Pipe()
	done := make(chan error, 1)
	t.Cleanup(func() {
		cancel()
		// Both endpoints may already be closed by normal service teardown.
		_ = a.Close()
		_ = b.Close()
		timer := time.NewTimer(6 * time.Second)
		defer timer.Stop()
		select {
		case err := <-done:
			if err != nil {
				t.Error(err)
			}
		case <-timer.C:
			t.Error("service failed to stop within cancellation grace")
		}
	})
	go func() { done <- cs.Serve(ctx, a, h) }()
	client, err := cs.Connect(ctx, b)
	if err != nil {
		t.Fatal(err)
	}
	return client, ctx
}

// exchangeInvocation returns the outputs of one fixture call without touching testing state.
func exchangeInvocation(ctx context.Context, client *cs.Client, v cs.Invocation, input []byte) ([]byte, []byte, cs.Completion, error) {
	stream, err := client.Invoke(ctx, v)
	if err != nil {
		return nil, nil, cs.Completion{}, err
	}
	var out, diag bytes.Buffer
	completion, err := cs.Exchange(ctx, stream, bytes.NewReader(input), &out, &diag)
	return out.Bytes(), diag.Bytes(), completion, err
}

func run(t *testing.T, ctx context.Context, client *cs.Client, v cs.Invocation, input []byte) ([]byte, []byte, cs.Completion) {
	t.Helper()
	out, diag, completion, err := exchangeInvocation(ctx, client, v, input)
	if err != nil {
		t.Fatal(err)
	}
	return out, diag, completion
}

// The same client scenario runs in process and against the Rust executable.
// Budget 20 seconds; all coordination is by protocol records or completion.
func fixtureScenario(t *testing.T, c *cs.Client, ctx context.Context) {
	info, err := c.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(info.Operations, servicetest.FixtureOperations()) {
		t.Fatal(info)
	}
	where := invocation("where")
	where.Args = []byte(`{"label":"here"}`)
	where.Env["PROBE"] = "value"
	location, _, _ := run(t, ctx, c, where, nil)
	expected := `{"label":"here","context":{"conversation":"c1","caller":{"kind":"user"},"locale":{"timeZone":"UTC","languages":["en-US"]}},"cwd":"/tmp","value":"value"}`
	if string(location) != expected {
		t.Fatalf("where bytes: %s", location)
	}
	var wg sync.WaitGroup
	for range 12 {
		wg.Go(func() {
			input := bytes.Repeat([]byte{0, 255, 128, 1}, 20000)
			out, _, completion, err := exchangeInvocation(ctx, c, invocation("echo"), input)
			if err != nil {
				t.Error(err)
				return
			}
			if !bytes.Equal(out, input) || completion.ExitCode != 0 {
				t.Error("binary echo failed")
			}
		})
	}
	wg.Wait()
	out, diag, completion := run(t, ctx, c, invocation("result"), nil)
	if string(out) != "command output" || string(diag) != "command diagnostic" || completion.ExitCode != 17 {
		t.Fatal(string(out), string(diag), completion)
	}
	v := invocation("result")
	v.Env["RESULT"] = "error"
	_, _, completion = run(t, ctx, c, v, nil)
	if completion.ExitCode != 1 || completion.Error == nil || completion.Error.Message != "command failed" {
		t.Fatal(completion)
	}
	s, err := c.Invoke(ctx, invocation("echo"))
	if err != nil {
		t.Fatal(err)
	}
	if r, err := s.Next(); err != nil || r.Kind != cs.InputPull {
		t.Fatal(r, err)
	}
	s.Cancel()
	run(t, ctx, c, invocation("result"), nil)
	run(t, ctx, c, invocation("retain"), nil)
	s, err = c.Conversation(ctx, cs.ConversationRequest{Operation: "status"})
	if err != nil {
		t.Fatal(err)
	}
	var held bytes.Buffer
	if _, err = cs.Exchange(ctx, s, bytes.NewReader(nil), &held, io.Discard); err != nil {
		t.Fatal(err)
	}
	status, err := cs.Decode[cs.ConversationStatus](held.Bytes())
	if err != nil || len(status.Conversations) != 1 || status.Conversations[0] != "c1" {
		t.Fatal(status, err)
	}
	numbers, err := c.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	var next uint64 = 1
	var mu sync.Mutex
	go func() {
		answered <- numbers.Answer(ctx, func(_ context.Context, r cs.NumbersRequest) (uint64, error) {
			mu.Lock()
			defer mu.Unlock()
			first := next
			next += uint64(r.Count)
			return first, nil
		})
	}()
	for _, want := range []string{`{"first":1}`, `{"first":2}`} {
		out, _, completion = run(t, ctx, c, invocation("number"), nil)
		if string(out) != want || completion.ExitCode != 0 {
			t.Fatal(string(out), completion)
		}
	}
	if err = c.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err = <-answered; err != nil {
		t.Fatal(err)
	}
}

func TestFixture(t *testing.T) {
	c, ctx := clientFor(t, servicetest.NewFixture())
	fixtureScenario(t, c, ctx)
}

type processConn struct {
	net.Conn
	in  io.WriteCloser
	out io.ReadCloser
}

func (p processConn) Read(b []byte) (int, error) { return p.out.Read(b) }

func (p processConn) Write(b []byte) (int, error) { return p.in.Write(b) }

func (p processConn) Close() error {
	return errors.Join(p.in.Close(), p.out.Close())
}

func TestRustFixture(t *testing.T) {
	path := os.Getenv("DEMI_RUST_FIXTURE")
	if path == "" {
		t.Skip("DEMI_RUST_FIXTURE is unset")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	command := exec.CommandContext(ctx, path)
	input, err := command.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// Process and transport teardown can both close these pipes.
		_ = input.Close()
	})
	output, err := command.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// Wait and transport teardown can both close the output pipe.
		_ = output.Close()
	})
	command.Stderr = os.Stderr
	if err = command.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if t.Failed() {
			cancel()
		}
		if err := command.Wait(); err != nil {
			t.Errorf("Rust fixture exit: %v", err)
		}
	})
	client, err := cs.Connect(ctx, processConn{in: input, out: output})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// Normal shutdown may have closed the peer before the transport is released.
		_ = client.Close()
	})
	fixtureScenario(t, client, ctx)
}

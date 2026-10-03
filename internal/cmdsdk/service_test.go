package cmdsdk

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"os"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/commandwire"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}
func invocation(op string) commandwire.Invocation {
	return commandwire.Invocation{Operation: op, InvocationID: op, Args: []byte(`{}`), Cwd: "/tmp", Env: map[string]string{}, Context: commandwire.CommandContext{Conversation: "one", Caller: &commandwire.AgentCaller{Number: 1}, Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}}}
}

type fixture struct{}

func (fixture) Operations() []string { return []string{"echo", "hold", "short", "flood"} }
func (fixture) Invoke(ctx context.Context, c InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	switch c.Request.Operation {
	case "echo", "hold":
		for {
			b, err := c.Input.Next(ctx)
			if errors.Is(err, io.EOF) {
				if c.Request.Operation == "echo" {
					return commandwire.Completion{ExitCode: 7}, c.Output.Stderr(ctx, []byte("done"))
				}
				return commandwire.Completion{}, nil
			}
			if err != nil {
				return commandwire.Completion{}, err
			}
			if c.Request.Operation == "echo" {
				if err = c.Output.Stdout(ctx, b); err != nil {
					return commandwire.Completion{}, err
				}
			}
		}
	case "flood":
		for {
			if err := c.Output.Stdout(ctx, bytes.Repeat([]byte{7}, commandwire.MaxRecordBytes)); err != nil {
				return commandwire.Completion{}, err
			}
		}
	default:
		return commandwire.Completion{}, c.Output.Stdout(ctx, []byte("ok"))
	}
}
func connected(t *testing.T, h Handler[commandwire.Invocation]) (*Client, <-chan error) {
	t.Helper()
	left, right := net.Pipe()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		done <- Serve(ctx, right, h)
		close(done)
	}()
	client, err := Connect(ctx, left)
	must(t, err)
	t.Cleanup(func() {
		cancel()
		_ = client.Close()
		select {
		case err := <-done:
			if err != nil {
				t.Error(err)
			}
		default:
			<-done
		}
	})
	return client, done
}
func completed(t *testing.T, o *CommandOutput) []byte {
	t.Helper()
	var out []byte
	seen := false
	for {
		r, err := o.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		must(t, err)
		switch r := r.(type) {
		case commandwire.Stdout:
			out = append(out, r...)
		case commandwire.Stderr:
			t.Fatalf("stderr %q", r)
		case commandwire.InputPull:
			t.Fatal("unexpected input pull")
		case commandwire.Completed:
			seen = true
			if r.Completion.ExitCode != 0 || r.Completion.Error != nil {
				t.Fatalf("completion: %+v", r.Completion)
			}
		}
	}
	if !seen {
		t.Fatal("missing completion")
	}
	return out
}
func short(t *testing.T, c *Client) {
	t.Helper()
	_, o, err := c.Invoke(t.Context(), invocation("short"))
	must(t, err)
	if string(completed(t, o)) != "ok" {
		t.Fatal("short output")
	}
}

// In-memory protocol scenarios cost no external processes and have a 30 s suite budget.
func TestHeldCallsBeyondAnyCountAllStartAndFinish(t *testing.T) {
	c, _ := connected(t, fixture{})
	inputs := make([]*CommandInput, 256)
	outputs := make([]*CommandOutput, 256)
	for k := range inputs {
		var err error
		inputs[k], outputs[k], err = c.Invoke(t.Context(), invocation("hold"))
		must(t, err)
	}
	var finished sync.WaitGroup
	for k := range inputs {
		finished.Go(func() {
			record, err := outputs[k].Next(t.Context())
			must(t, err)
			if _, ok := record.(commandwire.InputPull); !ok {
				t.Fatalf("expected input pull, got %T", record)
			}
			must(t, inputs[k].End())
			completed(t, outputs[k])
		})
	}
	finished.Wait()
	short(t, c)
}
func TestCancellingCallNeverTurnsAwayNext(t *testing.T) {
	c, _ := connected(t, fixture{})
	held := make([]*CommandInput, 64)
	for k := range held {
		var err error
		held[k], _, err = c.Invoke(t.Context(), invocation("hold"))
		must(t, err)
		defer held[k].Cancel()
	}
	for k := range 200 {
		held[k%len(held)].Cancel()
		i, _, err := c.Invoke(t.Context(), invocation("hold"))
		must(t, err)
		held[k%len(held)] = i
		defer i.Cancel()
	}
	short(t, c)
}
func TestUnreadOutputsNeverHoldBackIndependentCall(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _ := connected(t, fixture{})
		for range 64 {
			_, o, err := c.Invoke(t.Context(), invocation("flood"))
			must(t, err)
			record, err := o.Next(t.Context())
			must(t, err)
			if _, ok := record.(commandwire.Stdout); !ok {
				t.Fatalf("expected stdout, got %T", record)
			}
		}
		synctest.Wait()
		short(t, c)
	})
}
func TestAbandoningBurstKeepsConnection(t *testing.T) {
	c, _ := connected(t, fixture{})
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	var wg sync.WaitGroup
	started := make(chan struct{}, 1000)
	for range 1000 {
		wg.Go(func() {
			started <- struct{}{}
			i, _, err := c.Invoke(ctx, invocation("hold"))
			if err == nil {
				i.Cancel()
			} else if !errors.Is(err, context.Canceled) {
				t.Error(err)
			}
		})
	}
	for range 1000 {
		<-started
	}
	cancel()
	wg.Wait()
	short(t, c)
}

// observedCancellation exposes the handler's exit after its stream is reset.
type observedCancellation struct {
	fixture
	operation string
	cancelled chan struct{}
}

func (h *observedCancellation) Invoke(ctx context.Context, c InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	completion, err := h.fixture.Invoke(ctx, c)
	if c.Request.Operation == h.operation && ctx.Err() != nil {
		close(h.cancelled)
	}
	return completion, err
}
func TestConcurrentBinaryEchoAndCancelPreserveConnection(t *testing.T) {
	h := &observedCancellation{operation: "hold", cancelled: make(chan struct{})}
	c, _ := connected(t, h)
	i, held, err := c.Invoke(t.Context(), invocation("hold"))
	must(t, err)
	record, err := held.Next(t.Context())
	must(t, err)
	if _, ok := record.(commandwire.InputPull); !ok {
		t.Fatalf("held record %T", record)
	}
	var wg sync.WaitGroup
	for range 12 {
		wg.Go(func() {
			i, o, err := c.Invoke(t.Context(), invocation("echo"))
			if err != nil {
				t.Error(err)
				return
			}
			source := &chunks{values: [][]byte{{0, 255, 1}, bytes.Repeat([]byte{128}, 64000), []byte("end")}}
			sink := &capture{}
			result, err := (Exchange{Input: i, Output: o}).Run(t.Context(), source, sink)
			if err != nil {
				t.Error(err)
				return
			}
			expected := append([]byte{0, 255, 1}, bytes.Repeat([]byte{128}, 64000)...)
			expected = append(expected, []byte("end")...)
			if result.ExitCode != 7 || !bytes.Equal(sink.stdout, expected) || string(sink.stderr) != "done" {
				t.Error("binary echo differs")
			}
		})
	}
	wg.Wait()
	i.Cancel()
	<-h.cancelled
	short(t, c)
}
func TestResetInterruptsFlowControlBlockedOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		h := &observedCancellation{operation: "flood", cancelled: make(chan struct{})}
		c, _ := connected(t, h)
		i, o, err := c.Invoke(t.Context(), invocation("flood"))
		must(t, err)
		_, err = o.Next(t.Context())
		must(t, err)
		synctest.Wait()
		i.Cancel()
		<-h.cancelled
		short(t, c)
	})
}

// The peer answers conversations from headers and invocations after metadata,
// leaving later request bytes blocked behind a 16-byte HTTP/2 stream window.
func TestInputAfterEarlyAnswerIsNotFailure(t *testing.T) {
	left, right := net.Pipe()
	listener := &oneListener{conn: right, closed: make(chan struct{})}
	server := &http.Server{Protocols: protocols(), HTTP2: &http.HTTP2Config{MaxReceiveBufferPerStream: 16}, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == commandwire.InvokePath {
			metadata, err := readChunk(r.Body, commandwire.MaxMetadataBytes)
			if err != nil {
				t.Error(err)
				return
			}
			if _, err := commandwire.DecodeInvocation(metadata); err != nil {
				t.Error(err)
				return
			}
		}
		for _, record := range []commandwire.Record{commandwire.Stdout("{}"), commandwire.Completed{Completion: commandwire.Completion{}}} {
			data, err := commandwire.EncodeRecord(record)
			if err != nil {
				t.Error(err)
				return
			}
			if _, err := w.Write(data); err != nil {
				t.Error(err)
				return
			}
		}
	})}
	served := make(chan error, 1)
	go func() { served <- server.Serve(listener) }()
	defer func() {
		must(t, server.Close())
		if err := <-served; !errors.Is(err, http.ErrServerClosed) {
			t.Error(err)
		}
	}()
	c, err := Connect(t.Context(), left)
	must(t, err)
	defer func() { must(t, c.Close()) }()
	for _, conversation := range []string{"first", "second"} {
		_, output, err := c.Conversation(t.Context(), &commandwire.ConversationRelease{Conversation: conversation})
		must(t, err)
		for _, want := range []commandwire.Record{commandwire.Stdout("{}"), commandwire.Completed{Completion: commandwire.Completion{}}} {
			got, err := output.Next(t.Context())
			must(t, err)
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("record %#v, want %#v", got, want)
			}
		}
		if _, err := output.Next(t.Context()); !errors.Is(err, io.EOF) {
			t.Fatalf("end: %v", err)
		}
	}
	i, o, err := c.Invoke(t.Context(), invocation("echo"))
	must(t, err)
	if got := string(completed(t, o)); got != "{}" {
		t.Fatal(got)
	}
	must(t, i.Write(t.Context(), make([]byte, 1024)))
	must(t, i.End())
}

type chunks struct{ values [][]byte }

func (c *chunks) Next(context.Context) ([]byte, error) {
	if len(c.values) == 0 {
		return nil, io.EOF
	}
	b := c.values[0]
	c.values = c.values[1:]
	return b, nil
}

type capture struct{ stdout, stderr []byte }

func (c *capture) Stdout(_ context.Context, b []byte) error {
	c.stdout = append(c.stdout, b...)
	return nil
}
func (c *capture) Stderr(_ context.Context, b []byte) error {
	c.stderr = append(c.stderr, b...)
	return nil
}

func TestMetadataAndBinaryInputInOneBody(t *testing.T) {
	c, _ := connected(t, fixture{})
	metadata, err := commandwire.EncodeMetadata(invocation("echo"))
	must(t, err)
	binary := []byte{0, 255, 13, 10, 128}
	input, err := commandwire.EncodeInput(binary)
	must(t, err)
	_, o, err := c.invokeAt(t.Context(), commandwire.InvokePath, append(metadata, input...), true)
	must(t, err)
	var records []commandwire.Record
	for {
		record, err := o.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		must(t, err)
		records = append(records, record)
	}
	want := []commandwire.Record{commandwire.InputPull{}, commandwire.Stdout(binary), commandwire.Stderr("done"), commandwire.Completed{Completion: commandwire.Completion{ExitCode: 7}}}
	if !reflect.DeepEqual(records, want) {
		t.Fatalf("records: %#v, want %#v", records, want)
	}
}

func TestOwnedFilePipesCarryServiceProtocol(t *testing.T) {
	serverRead, clientWrite, err := os.Pipe()
	must(t, err)
	clientRead, serverWrite, err := os.Pipe()
	if err != nil {
		_ = serverRead.Close()
		_ = clientWrite.Close()
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() { done <- Serve(ctx, &PipeConn{Reader: serverRead, Writer: serverWrite}, fixture{}); close(done) }()
	c, err := Connect(ctx, &PipeConn{Reader: clientRead, Writer: clientWrite})
	if err != nil {
		cancel()
		<-done
		t.Fatal(err)
	}
	t.Cleanup(func() { cancel(); _ = c.Close(); <-done })
	short(t, c)
	must(t, c.Shutdown(t.Context()))
	must(t, <-done)
}

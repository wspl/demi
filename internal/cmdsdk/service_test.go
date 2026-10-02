package cmdsdk

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"os"
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
		case commandwire.Completed:
			seen = true
			if r.Completion.ExitCode != 0 {
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
	for k := range inputs {
		record, err := outputs[k].Next(t.Context())
		must(t, err)
		if _, ok := record.(commandwire.InputPull); !ok {
			t.Fatalf("expected input pull, got %T", record)
		}
		must(t, inputs[k].End())
		completed(t, outputs[k])
	}
	short(t, c)
}
func TestCancellingCallNeverTurnsAwayNext(t *testing.T) {
	c, _ := connected(t, fixture{})
	for range 200 {
		i, _, err := c.Invoke(t.Context(), invocation("hold"))
		must(t, err)
		i.Cancel()
	}
	short(t, c)
}
func TestUnreadOutputsNeverHoldBackIndependentCall(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _ := connected(t, fixture{})
		for range 64 {
			_, o, err := c.Invoke(t.Context(), invocation("flood"))
			must(t, err)
			_, err = o.Next(t.Context())
			must(t, err)
		}
		synctest.Wait()
		short(t, c)
	})
}
func TestAbandoningBurstKeepsConnection(t *testing.T) {
	c, _ := connected(t, fixture{})
	var wg sync.WaitGroup
	for range 1000 {
		wg.Go(func() {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			i, _, err := c.Invoke(ctx, invocation("hold"))
			if err == nil {
				i.Cancel()
			} else {
				t.Error(err)
			}
		})
	}
	wg.Wait()
	short(t, c)
}
func TestConcurrentBinaryEchoAndCancelPreserveConnection(t *testing.T) {
	c, _ := connected(t, fixture{})
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
	i, _, err := c.Invoke(t.Context(), invocation("hold"))
	must(t, err)
	i.Cancel()
	short(t, c)
}
func TestResetInterruptsFlowControlBlockedOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _ := connected(t, fixture{})
		i, o, err := c.Invoke(t.Context(), invocation("flood"))
		must(t, err)
		_, err = o.Next(t.Context())
		must(t, err)
		synctest.Wait()
		i.Cancel()
		short(t, c)
	})
}
func TestInputAfterEarlyAnswerIsNotFailure(t *testing.T) {
	c, _ := connected(t, fixture{})
	i, o, err := c.Invoke(t.Context(), invocation("short"))
	must(t, err)
	completed(t, o)
	must(t, i.Write(t.Context(), []byte("late")))
	must(t, i.End())
	short(t, c)
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
	input, o, err := c.Invoke(t.Context(), invocation("echo"))
	must(t, err)
	defer input.Cancel()
	binary := []byte{0, 255, 13, 10, 128}
	pulls := 0
	var output, diagnostic []byte
	var code uint8
	for {
		r, err := o.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		must(t, err)
		switch r := r.(type) {
		case commandwire.Stdout:
			output = append(output, r...)
		case commandwire.Stderr:
			diagnostic = append(diagnostic, r...)
		case commandwire.Completed:
			code = r.Completion.ExitCode
		case commandwire.InputPull:
			pulls++
			switch pulls {
			case 1:
				must(t, input.Write(t.Context(), binary))
			case 2:
				must(t, input.End())
			default:
				t.Fatalf("pull after input ended: %d", pulls)
			}
		}
	}
	if pulls != 2 {
		t.Fatalf("got %d pulls, want one chunk pull and one EOF pull", pulls)
	}
	if !bytes.Equal(output, binary) || string(diagnostic) != "done" || code != 7 {
		t.Fatalf("output %v stderr %q code %d", output, diagnostic, code)
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

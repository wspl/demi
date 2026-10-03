package engine

import (
	"context"
	"errors"
	"io"
	"os"
	"runtime"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

type recordingHandler struct {
	mu       sync.Mutex
	requests []process.RawCommand
}

func (*recordingHandler) Operations() []string { return []string{process.Raw} }
func (h *recordingHandler) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error) {
	raw, err := process.DecodeRawCommand(invocation.Request.Args)
	if err != nil {
		return commandwire.Completion{}, err
	}
	h.mu.Lock()
	h.requests = append(h.requests, raw)
	h.mu.Unlock()
	for {
		b, err := invocation.Input.Next(ctx)
		if len(b) > 0 {
			if err := invocation.Output.Stdout(ctx, b); err != nil {
				return commandwire.Completion{}, err
			}
		}
		if errors.Is(err, io.EOF) {
			return commandwire.Completion{ExitCode: 7}, nil
		}
		if err != nil {
			return commandwire.Completion{}, err
		}
	}
}
func TestADeclaredCommandReachesTheJobsHandler(t *testing.T) {
	root := t.TempDir()
	handler := &recordingHandler{}
	contextID := "0123456789abcdef0123456789abcdef"
	job, err := StartJob(t.Context(), process.JobStart{Script: `/usr/bin/env; printf body | fixture --flag; echo " $?"`, Cwd: root, Env: map[string]string{"HOME": root, "PATH": os.Getenv("PATH")}, Commands: &process.JobCommands{Context: contextID, Roots: []string{"fixture"}, Handler: handler}}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		job.Cancel()
		job.Wait(context.Background())
	}()
	var output strings.Builder
	for chunk := range job.Output() {
		if chunk.Stream == runnerwire.Stdout {
			output.Write(chunk.Bytes)
		}
	}
	status, _ := job.Wait(t.Context())
	if status.Code == nil || *status.Code != 0 || !strings.Contains(output.String(), "DEMI_CONTEXT_ID="+contextID+"\n") || !strings.HasSuffix(output.String(), "body 7\n") {
		t.Fatalf("exit %+v output %q", status, output.String())
	}
	handler.mu.Lock()
	defer handler.mu.Unlock()
	if len(handler.requests) != 1 {
		t.Fatalf("requests: %v", handler.requests)
	}
	asked := handler.requests[0]
	if asked.Context != contextID || asked.Root != "fixture" || asked.Live || len(asked.Argv) != 1 || asked.Argv[0] != "--flag" {
		t.Fatalf("request: %+v", asked)
	}
}

// A closed consumer must become a shell status, leaving the next statement runnable.
func TestDeclaredBrokenPipeExits141(t *testing.T) {
	root := t.TempDir()
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := reader.Close(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = writer.Close() }() // Execute may close cancelled job streams.
	result, output, diagnostic := shellFiles(t, root, `fixture; printf '%s' "$?" >&2`, func(o *Options) {
		o.Stdout = writer
		o.Commands = &process.JobCommands{Context: "0123456789abcdef0123456789abcdef", Roots: []string{"fixture"}, Handler: &recordingHandler{}}
		if _, err := o.Stdin.WriteString("body"); err != nil {
			t.Fatal(err)
		}
		if _, err := o.Stdin.Seek(0, 0); err != nil {
			t.Fatal(err)
		}
	})
	if result.Code != 0 || output != "" || diagnostic != "141" {
		t.Fatalf("result %+v output %q diagnostic %q", result, output, diagnostic)
	}
}

// stoppedInputHandler joins its input worker before returning, like an RPC whose
// remote command finishes without needing stdin.
type stoppedInputHandler struct {
	waiting chan struct{}
	waits   int
	writer  *os.File
}

func (*stoppedInputHandler) Operations() []string { return []string{process.Raw} }
func (*stoppedInputHandler) Check()               {}
func (h *stoppedInputHandler) Waiting(delta int) {
	if delta == 1 {
		h.waiting <- struct{}{}
	}
}
func (h *stoppedInputHandler) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error) {
	inputCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := invocation.Input.Next(inputCtx)
		done <- err
	}()
	for range h.waits {
		select {
		case <-h.waiting:
		case <-ctx.Done():
		}
	}
	cancel()
	err := <-done
	if !errors.Is(err, context.Canceled) {
		return commandwire.Completion{}, errors.New("input read did not return cancellation")
	}
	if ctx.Err() != nil {
		return commandwire.Completion{}, errors.New("input read waited for outer command cancellation")
	}
	// Only supply bytes after the canceled read has ended. The next shell
	// command must receive them, with no abandoned reader consuming them.
	_, err = h.writer.WriteString("still readable\n")
	return commandwire.Completion{}, err
}

// One in-process shell and pipe; normally finishes in milliseconds. The outer
// context is only a deadlock watchdog, including when run against the old code.
func TestDeclaredInputCancellationPreservesShellInput(t *testing.T) {
	for _, test := range []struct {
		name   string
		script string
		waits  int
	}{
		{"pipe", `fixture; read -r line; printf '%s' "$line"`, 1},
		// The producer cannot write until the handler has canceled and joined
		// its input pull. Wait for both that pull and the producer's read.
		// The following read uses the same substitution input.
		{"process substitution", `{ fixture; read -r line; printf '%s' "$line"; } < <(read -r produced; printf '%s\n' "$produced")`, 2},
	} {
		t.Run(test.name, func(t *testing.T) {
			if test.name == "process substitution" && runtime.GOOS == "windows" {
				t.Skip("process substitution requires Unix FIFOs")
			}
			reader, writer, err := os.Pipe()
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = reader.Close() }() // Execute may close canceled job streams.
			defer func() { _ = writer.Close() }()
			handler := &stoppedInputHandler{waiting: make(chan struct{}, test.waits), waits: test.waits, writer: writer}
			result, output, diagnostic := shellFiles(t, t.TempDir(), test.script, func(o *Options) {
				o.Stdin = reader
				o.Observe = handler
				o.Commands = &process.JobCommands{Context: "0123456789abcdef0123456789abcdef", Roots: []string{"fixture"}, Handler: handler}
			})
			if result.Code != 0 || output != "still readable" || diagnostic != "" {
				t.Fatalf("result %+v output %q diagnostic %q", result, output, diagnostic)
			}
		})
	}
}

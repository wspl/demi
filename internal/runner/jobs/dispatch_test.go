package jobs_test

import (
	"context"
	"fmt"
	"io"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/jobs/jobstest"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
)

// dispatchFixture exposes the real dispatcher through its owned local endpoint.
func dispatchFixture(t *testing.T) (*jobstest.Dispatch, *jobs.ExecutionContext, *jobstest.ContextRegistration) {
	t.Helper()
	tree, err := cmddecl.DecodeDeclaration(
		[]byte(
			`{"name":"fixture","summary":"Test callback.","kind":"rpc","runningHint":"Working",` +
				`"input":{"type":"object","properties":{"body":{"type":"string"}},"required":["body"]},"stdinField":"body"}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := runnerproto.BuildManifest([]cmddecl.Node[cmddecl.NativeOperation]{tree}, nil)
	if err != nil {
		t.Fatal(err)
	}
	value, err := manifest.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	backend, err := runnerproto.ParseBackendURL("http://127.0.0.1:1")
	if err != nil {
		t.Fatal(err)
	}
	pipes, err := process.NewPipeClient(backend, func() (runnerproto.DeviceToken, bool) {
		return "", false
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := pipes.Close(); err != nil {
			t.Error(err)
		}
	})
	fixture := jobstest.NewDispatch(t.Context(), t, t.TempDir(), value, pipes)
	command := cmdproto.Context{
		Conversation: "conversation",
		Caller:       &cmdproto.AgentCaller{Number: 1},
		Locale:       cmdproto.CommandLocale{TimeZone: "UTC", Languages: []cmdproto.LanguageTag{"en-US"}},
	}
	execution, registration, err := fixture.Context(t.Context(), "job", command)
	if err != nil {
		t.Fatal(err)
	}
	return fixture, execution, registration
}

func dispatchRequest(t *testing.T, execution *jobs.ExecutionContext, argv ...string) cmdproto.LocalInvocation {
	t.Helper()
	raw, err := process.NewRawCommand(execution.ID, "fixture", argv, true)
	if err != nil {
		t.Fatal(err)
	}
	args, err := raw.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	request := localRequest()
	request.Operation = process.Raw
	request.Args = args
	return request
}

func dispatchMessage(t *testing.T, fixture *jobstest.Dispatch) runnerproto.Outbound {
	t.Helper()
	message, err := runnerproto.DecodeOutbound(<-fixture.Outgoing)
	if err != nil {
		t.Fatal(err)
	}
	return message
}

func TestHelpNeverReadsStdinOrCallsBackend(t *testing.T) {
	fixture, execution, _ := dispatchFixture(t)
	stdout := &outputBuffer{}
	result, err := process.Forward(
		t.Context(),
		fixture.Server.Endpoint(),
		dispatchRequest(t, execution, "--help"),
		process.Stdio{Stdin: neverRead{t: t}, Stdout: stdout, Stderr: &outputBuffer{}},
	)
	if err != nil || result.ExitCode != 0 || !strings.HasPrefix(stdout.String(), "fixture: Test callback.") {
		t.Fatalf("help: %+v %v %q", result, err, stdout.String())
	}
	select {
	case frame := <-fixture.Outgoing:
		t.Fatalf("help sent backend request: %x", frame)
	default:
	}
}

func TestCallbackExitClearsHintAndRevokedContextCannotDispatch(t *testing.T) {
	fixture, execution, registration := dispatchFixture(t)
	request := dispatchRequest(t, execution)
	done := make(chan error, 1)
	go func() {
		result, err := process.Forward(
			t.Context(),
			fixture.Server.Endpoint(),
			request,
			process.Stdio{Stdin: neverRead{t: t}, Stdout: &outputBuffer{}, Stderr: &outputBuffer{}},
		)
		if err == nil && result.ExitCode != 7 {
			err = fmt.Errorf("exit %d", result.ExitCode)
		}
		done <- err
	}()
	hint, ok := dispatchMessage(t, fixture).(*runnerproto.JobRunningHint)
	if !ok || hint.Hint == nil || *hint.Hint != "Working" {
		t.Fatal("missing hint")
	}
	call, ok := dispatchMessage(t, fixture).(*runnerproto.RPCCall)
	if !ok || string(call.Args) != `{"body":""}` {
		t.Fatalf("callback %+v", call)
	}
	if err := fixture.Deliver(t.Context(), &runnerproto.RPCExit{CallID: call.CallID, ExitCode: 7}); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	clearHint, ok := dispatchMessage(t, fixture).(*runnerproto.JobRunningHint)
	if !ok || clearHint.Hint != nil || clearHint.InvocationID != hint.InvocationID {
		t.Fatal("hint not cleared")
	}
	if err := registration.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	result, err := process.Forward(
		t.Context(),
		fixture.Server.Endpoint(),
		request,
		process.Stdio{Stdin: neverRead{t: t}, Stdout: &outputBuffer{}, Stderr: &outputBuffer{}},
	)
	if err != nil || result.ExitCode == 0 {
		t.Fatalf("revoked invocation: %+v %v", result, err)
	}
	select {
	case <-fixture.Outgoing:
		t.Fatal("revoked context called backend")
	default:
	}
}

func TestCancellationCancelsCallbackAndClearsHint(t *testing.T) {
	fixture, execution, _ := dispatchFixture(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	request := dispatchRequest(t, execution)
	done := make(chan error, 1)
	go func() {
		_, err := process.Forward(
			ctx,
			fixture.Server.Endpoint(),
			request,
			process.Stdio{Stdin: neverRead{t: t}, Stdout: &outputBuffer{}, Stderr: &outputBuffer{}},
		)
		done <- err
	}()
	if _, ok := dispatchMessage(t, fixture).(*runnerproto.JobRunningHint); !ok {
		t.Fatal("missing hint")
	}
	if _, ok := dispatchMessage(t, fixture).(*runnerproto.RPCCall); !ok {
		t.Fatal("missing callback")
	}
	cancel()
	if err := <-done; err == nil {
		t.Fatal("cancelled call succeeded")
	}
	if _, ok := dispatchMessage(t, fixture).(*runnerproto.RPCCancel); !ok {
		t.Fatal("missing RPC cancellation")
	}
	if hint, ok := dispatchMessage(t, fixture).(*runnerproto.JobRunningHint); !ok || hint.Hint != nil {
		t.Fatal("hint not cleared")
	}
}

func TestBackendCommandsAreNeverTurnedAway(t *testing.T) {
	// Exercises 200 concurrent local clients; no real backend or model is involved.
	fixture, execution, _ := dispatchFixture(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	request := dispatchRequest(t, execution)
	const clients = 200
	ended := make(chan error, clients)
	var workers sync.WaitGroup
	for range clients {
		workers.Add(1)
		go func() {
			defer workers.Done()
			_, err := process.Forward(
				ctx,
				fixture.Server.Endpoint(),
				request,
				process.Stdio{Stdin: neverRead{t: t}, Stdout: &outputBuffer{}, Stderr: &outputBuffer{}},
			)
			ended <- err
		}()
	}
	drained := make(chan struct{})
	reached := make(chan struct{})
	go func() {
		defer close(drained)
		count := 0
		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-fixture.Outgoing:
				message, err := runnerproto.DecodeOutbound(frame)
				if err != nil {
					t.Error(err)
					return
				}
				if _, ok := message.(*runnerproto.RPCCall); ok {
					count++
					if count == clients {
						close(reached)
					}
				}
			}
		}
	}()
	select {
	case err := <-ended:
		cancel()
		workers.Wait()
		<-drained
		t.Fatalf("call refused while backend held it: %v", err)
	case <-reached:
	}
	cancel()
	workers.Wait()
	<-drained
}

// Invalid finite stdin is rejected before dispatch; only a local socket is used.
func TestFiniteStdinReportsUTF8Detail(t *testing.T) {
	fixture, execution, _ := dispatchFixture(t)
	raw, err := process.NewRawCommand(execution.ID, "fixture", nil, false)
	if err != nil {
		t.Fatal(err)
	}
	args, err := raw.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	request := localRequest()
	request.Operation = process.Raw
	request.Args = args
	stderr := &outputBuffer{}
	result, err := process.Forward(
		t.Context(),
		fixture.Server.Endpoint(),
		request,
		process.Stdio{Stdin: io.NopCloser(strings.NewReader("ok\xff")), Stdout: &outputBuffer{}, Stderr: stderr},
	)
	if err != nil || result.ExitCode != 1 {
		t.Fatalf("invalid stdin: %+v %v", result, err)
	}
	if got, want := stderr.String(), "demi-runner: invalid utf-8 sequence of 1 bytes from index 2\n"; got != want {
		t.Fatalf("stderr = %q, want %q", got, want)
	}
	select {
	case frame := <-fixture.Outgoing:
		t.Fatalf("invalid stdin reached backend: %x", frame)
	default:
	}
}

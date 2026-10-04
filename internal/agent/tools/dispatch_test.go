package tools

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

type dispatchHost struct{ host.Host }

func (*dispatchHost) Key() host.Key {
	return "test"
}

type dispatchProduct struct {
	environment *dispatchEnvironment
	resolutions int
	creations   int
	scope       EnvironmentScope
	failure     error
}

func (p *dispatchProduct) Host(_ context.Context, _ NodeContext) (*dispatchHost, error) {
	p.resolutions++
	return &dispatchHost{}, p.failure
}

func (p *dispatchProduct) Create(
	_ context.Context,
	scope EnvironmentScope,
	_ *dispatchHost,
) (host.ShellEnvironment, error) {
	p.creations++
	p.scope = scope
	return p.environment, nil
}

type dispatchEnvironment struct {
	ownedEnvironment
	request  host.ExecRequest
	status   host.CommandStatus
	stdin    string
	aborted  bool
	released []types.CommandID
	failure  error
}

func (e *dispatchEnvironment) Exec(_ context.Context, request host.ExecRequest) (host.CommandStatus, error) {
	e.request = request
	return e.status, e.failure
}

func (e *dispatchEnvironment) Status(_ types.CommandID) (host.CommandStatus, error) {
	return e.status, e.failure
}

func (e *dispatchEnvironment) Write(_ context.Context, _ types.CommandID, stdin []byte) error {
	e.stdin = string(stdin)
	return e.failure
}

func (e *dispatchEnvironment) Abort(_ context.Context, _ types.CommandID) error {
	e.aborted = true
	e.status.State.Phase = host.Aborted
	return e.failure
}

func (e *dispatchEnvironment) ReleaseCommand(_ context.Context, command types.CommandID) bool {
	e.released = append(e.released, command)
	return true
}

func TestDispatchUsesCurrentHostAndReleasesEndedHandles(t *testing.T) {
	environment := &dispatchEnvironment{status: exited("hello\n")}
	product := &dispatchProduct{environment: environment}
	environments := &Environments{}
	defer func() {
		if err := environments.Dispose(context.WithoutCancel(t.Context())); err != nil {
			t.Error(err)
		}
	}()
	access := ShellAccess[*dispatchHost]{
		Hosts:        product,
		Shells:       product,
		Environments: environments,
		Context:      NodeContext{Root: "root", Node: "child", CWD: "/work"},
		Agent:        2,
	}
	call := session.ToolInvocation{
		ToolName:   "shell_exec",
		ToolUseID:  "call",
		Input:      []byte(`{"script":"echo hello","shellId":3,"timeoutMs":7}`),
		Generation: 4,
		Model:      storetest.TestModel(),
	}
	outcome, err := access.Invoke(t.Context(), call)
	if err != nil {
		t.Fatal(err)
	}
	if outcome.IsError || len(environment.released) != 1 || environment.released[0] != "17" {
		t.Fatal("ended command was not reported and released")
	}
	if environment.request.Script != "echo hello" || environment.request.Shell.ID != "3" ||
		environment.request.Caller.Node != "child" ||
		environment.request.Caller.Generation != 4 ||
		environment.request.ToolUseID != "call" ||
		environment.request.Window.Duration().Milliseconds() != 7 {
		t.Fatalf("wrong exec request: %+v", environment.request)
	}
	if product.scope.Agent != 2 || product.scope.Root != "root" || product.scope.Node != "child" {
		t.Fatalf("wrong scope: %+v", product.scope)
	}
	environment.status.Whole = nil
	environment.status.State = host.CommandState{Phase: host.Running}
	for _, tool := range []string{"shell_status", "shell_write", "shell_abort"} {
		call.ToolName = tool
		call.Input = []byte(`{"commandId":17}`)
		if tool == "shell_write" {
			call.Input = []byte(`{"commandId":17,"stdin":"yes\n"}`)
		}
		outcome, err = access.Invoke(t.Context(), call)
		if err != nil || outcome.IsError {
			t.Fatalf("%s failed", tool)
		}
	}
	if product.resolutions != 4 || product.creations != 1 || environment.stdin != "yes\n" || !environment.aborted ||
		len(environment.released) != 2 {
		t.Fatal("dispatch did not resolve each call and share the environment")
	}
	call.ToolName = "yield"
	call.Input = []byte(`{"durationMs":11}`)
	outcome, err = access.Invoke(t.Context(), call)
	effect, ok := outcome.Effect.(*session.ScheduleYield)
	if err != nil || !ok || effect.DurationMS != 11 || product.resolutions != 4 {
		t.Fatal("yield touched Host or lost its effect")
	}
	product.failure = errors.New("offline")
	call.ToolName = "shell_status"
	call.Input = []byte(`{"commandId":17}`)
	_, err = access.Invoke(t.Context(), call)
	if err == nil || err.Error() != "offline" {
		t.Fatal("Host failure did not become a tool failure")
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	product.failure = nil
	_, err = access.Invoke(ctx, call)
	if !errors.Is(err, context.Canceled) {
		t.Fatal("cancellation did not propagate")
	}
}

func TestPromptAndPageHistory(t *testing.T) {
	if strings.HasPrefix(SystemPrompt(" \n", "\t"), "\n") ||
		strings.Contains(SystemPrompt("", ""), "Registered commands:") {
		t.Fatal("blank prompt sections retained")
	}
	for _, phase := range []host.Phase{host.Running, host.Exited, host.Aborted} {
		view := host.PageView{
			ShellID:   "3",
			CommandID: "17",
			ToolUseID: "call",
			Tail:      "tail",
			Chars:     8,
			RunningMs: 2,
			State:     host.PageState{Phase: phase, ExitCode: 7},
		}
		frame := ShellOutput(nil, view).(*conversationproto.ShellOutputFrame)
		if frame.Status.Command().CommandID != "17" || frame.Status.Command().Tail != "tail" {
			t.Fatal("page view lost")
		}
		switch status := frame.Status.(type) {
		case *conversationproto.RunningStatus:
			if phase != host.Running {
				t.Fatal("wrong running state")
			}
		case *conversationproto.ExitedStatus:
			if phase != host.Exited || status.ExitCode != 7 {
				t.Fatal("wrong exit state")
			}
		case *conversationproto.AbortedStatus:
			if phase != host.Aborted {
				t.Fatal("wrong abort state")
			}
		}
	}
	blocks := []types.Block{
		&types.ToolCallBlock{
			View: &types.ShellView{
				ShellToolView: types.ShellToolView{CommandID: "17", Status: types.ShellViewStatusRunning},
			},
		},
		&types.ToolCallBlock{
			View: &types.ShellView{
				ShellToolView: types.ShellToolView{CommandID: "18", Status: types.ShellViewStatusRunning},
			},
		},
		&types.ToolCallBlock{
			View: &types.ShellView{
				ShellToolView: types.ShellToolView{CommandID: "17", Status: types.ShellViewStatusExited},
			},
		},
	}
	if diff := cmp.Diff([]types.CommandID{"18"}, StoredRunningCommands(blocks)); diff != "" {
		t.Fatal(diff)
	}
}

type failedNumbers struct {
	store.Tree
	failure error
}

func (s failedNumbers) NextNumber(context.Context, types.Sequence) (uint64, error) {
	return 0, s.failure
}

func TestStoreNumbersPreservesHostFailureAndCause(t *testing.T) {
	cause := errors.New("store unavailable")
	_, err := (StoreNumbers{Store: failedNumbers{failure: cause}}).Next(t.Context(), types.SequenceCommand)
	var failure *host.Error
	if !errors.Is(err, cause) || !errors.As(err, &failure) || failure.Kind != host.Failed ||
		err.Error() != cause.Error() {
		t.Fatalf("lost failure classification or cause: %v", err)
	}
}

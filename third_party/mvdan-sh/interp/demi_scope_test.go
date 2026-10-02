package interp_test

import (
	"bytes"
	"context"
	"strings"
	"testing"

	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

type scopeValue struct{ value string }

func (s *scopeValue) Clone() interp.ScopeState { return &scopeValue{value: s.value} }
func (*scopeValue) Check()                     {}
func (*scopeValue) Waiting(int)                {}

// TestHostBuiltinScopes proves host overrides work through command and builtin,
// and that changing subshell attributes cannot change the parent scope.
func TestHostBuiltinScopes(t *testing.T) {
	var output bytes.Buffer
	handler := func(ctx context.Context, args []string) error {
		hc := interp.HandlerCtx(ctx)
		state := hc.Scope().(*scopeValue)
		if len(args) > 1 {
			state.value = args[1]
			return nil
		}
		_, err := hc.Stdout.Write([]byte(state.value + "\n"))
		return err
	}
	r, err := interp.New(interp.StdIO(nil, &output, nil), interp.WithScopeState(&scopeValue{value: "initial"}), interp.Builtins(map[string]interp.ExecHandlerFunc{"umask": handler, "declared": handler}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`umask parent; (command umask child; builtin umask); umask; declared; type -t declared`), "scope")
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	if got := output.String(); got != "child\nparent\nparent\nbuiltin\n" {
		t.Fatalf("output %q", got)
	}
}

// TestHostExecScope proves a host builtin can exit only its current scope.
func TestHostExecScope(t *testing.T) {
	var output bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &output, nil), interp.Builtins(map[string]interp.ExecHandlerFunc{"exec": func(ctx context.Context, _ []string) error { return interp.HandlerCtx(ctx).Exit(interp.ExitStatus(7)) }}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`(exec); echo "subshell $?"; exec; echo unreachable`), "scope")
	if err != nil {
		t.Fatal(err)
	}
	err = r.Run(t.Context(), f)
	r.Wait()
	if code, ok := interp.IsExitStatus(err); !ok || code != 7 {
		t.Fatalf("exit: %v", err)
	}
	if output.String() != "subshell 7\n" {
		t.Fatalf("output %q", output.String())
	}
}

// TestWaitNextJoinsCompletedTask uses a handler event to establish completion.
func TestWaitNextJoinsCompletedTask(t *testing.T) {
	ready := make(chan struct{})
	var output bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &output, nil), interp.ExecHandler(func(ctx context.Context, args []string) error {
		if args[0] == "finish" {
			close(ready)
			return interp.ExitStatus(7)
		}
		<-ready
		return nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`finish & observe; wait -n; echo $?; wait -n; echo $?`), "wait")
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	if output.String() != "7\n127\n" {
		t.Fatalf("output %q", output.String())
	}
}

// TestBackgroundTerminationStatus proves task cancellation is a wait status,
// rather than a fatal cancellation of the shell which waits for it.
func TestBackgroundTerminationStatus(t *testing.T) {
	var output bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &output, nil), interp.Builtins(map[string]interp.ExecHandlerFunc{
		"terminate": func(ctx context.Context, args []string) error {
			return interp.HandlerCtx(ctx).TerminateBackground(args[1], 143)
		},
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`while :; do :; done & terminate "$!"; wait "$!"; echo $?; echo parent`), "background")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	if output.String() != "143\nparent\n" {
		t.Fatalf("output %q", output.String())
	}
}

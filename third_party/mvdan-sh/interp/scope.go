// Copyright (c) 2026, the Demi contributors
// See LICENSE for licensing information

package interp

import (
	"context"
	"fmt"
	"io"
	"maps"
	"strconv"
	"strings"
)

// ScopeState is host state that follows interpreter scopes. Clone must return
// independently mutable state for a subshell. Check and Waiting observe progress
// and interruptible reads; they must not block and may run concurrently on clones.
type ScopeState interface {
	Clone() ScopeState
	Check()
	Waiting(delta int)
}

// WithScopeState attaches host state without storing it in shell variables.
func WithScopeState(state ScopeState) RunnerOption {
	return func(r *Runner) error {
		r.scopeState = state
		return nil
	}
}

// Builtins registers host builtins, replacing native builtins with matching names.
// Functions may still shadow these names, as with native builtins. The map is
// copied, then shared immutably by subshells. Handlers must cooperate with ctx.
func Builtins(handlers map[string]ExecHandlerFunc) RunnerOption {
	return func(r *Runner) error {
		r.customBuiltins = maps.Clone(handlers)
		return nil
	}
}

func (r *Runner) isBuiltin(name string) bool {
	return r.customBuiltins[name] != nil || IsBuiltin(name)
}

// Scope returns the state of the scope invoking the handler.
func (hc HandlerContext) Scope() ScopeState { return hc.runner.scopeState }

// Descriptors returns a copy of the shell's descriptors above standard error.
// The interpreter retains ownership; a handler must not close them.
func (hc HandlerContext) Descriptors() map[string]io.ReadWriteCloser {
	return maps.Clone(hc.runner.extraFiles)
}

// NativeBuiltin invokes the original builtin, bypassing a host override.
func (hc HandlerContext) NativeBuiltin(ctx context.Context, args []string) error {
	exit := hc.runner.nativeBuiltin(ctx, hc.Pos, args[0], args[1:])
	if exit != (exitStatus{}) {
		return errBuiltinExitStatus(exit)
	}
	return nil
}

// Exit finishes the current shell with the result of a command. Subshell exit
// does not exit its parent. A host exec builtin uses this after running its command.
func (hc HandlerContext) Exit(err error) error {
	var exit exitStatus
	exit.fromHandlerError(err)
	exit.exiting = true
	return errBuiltinExitStatus(exit)
}

// RunScript runs an executable without a shebang as a script in a child scope.
// It preserves the host's handlers, ownership and scope state.
func (hc HandlerContext) RunScript(ctx context.Context, path string, args []string) error {
	return runScriptENOEXEC(ctx, hc, 0, path, args)
}

// BackgroundScope returns host state and cancellation for a shell background ID.
// The caller may inspect state concurrently only as its ScopeState permits.
func (hc HandlerContext) BackgroundScope(id string) (ScopeState, context.CancelFunc, error) {
	number, err := strconv.Atoi(strings.TrimPrefix(id, "g"))
	if err != nil || !strings.HasPrefix(id, "g") || number < 1 || number > len(hc.runner.bgProcs) {
		return nil, nil, fmt.Errorf("%s: no such job", id)
	}
	bg := hc.runner.bgProcs[number-1]
	return bg.state, bg.cancel, nil
}

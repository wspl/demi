// Package shell runs a single, independently cancellable runner shell job.
// The caller owns admission, live input queues, retained output and connection
// lifetime. Run owns the interpreter and the processes it starts.
package shell

import (
	"context"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/wspl/demi/go/commandservice"
	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// Sink receives output as it is produced, including before stdin reaches EOF.
// Write must stop when ctx ends, and must not retain p. Calls are serialized.
// G5c supplies the sink that retains output and publishes its bounded views.
type Sink interface {
	Write(ctx context.Context, stream string, p []byte) error
}

// Options describes one job. Env is the complete environment, not an overlay.
// The caller supplies HOME (including the device fallback) and runner-owned
// DEMI_* variables. Stdin is owned by Run and closed on every exit path; nil
// means EOF. An os.File avoids an eager reader that would consume live input.
type Options struct {
	Dir      string
	Env      map[string]string
	Stdin    *os.File
	Output   Sink
	Login    bool
	Live     bool
	Commands *Commands
	Recorder *commandservice.Recorder
}

// Result carries either Code, Signal or Err, and the shell's last directory.
// CancelCause requests a terminating signal; ordinary cancellation is SIGKILL.
type Result struct {
	Code   *uint8
	Signal string
	Err    error
	Dir    string
}

type CancelCause string

func (s CancelCause) Error() string { return string(s) }

// Run executes a fresh shell and waits for all its background work. A nonzero
// shell status is a known result, not an interpreter failure.
func Run(parent context.Context, script string, options Options) Result {
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	if options.Stdin == nil {
		input, err := openWaiting(ctx, os.DevNull)
		if err != nil {
			return Result{Err: err}
		}
		options.Stdin = input
	}
	defer options.Stdin.Close()
	state := &scope{options: options, cancel: cancel}
	output := &outputWriter{ctx: ctx, scope: state, stream: "stdout"}
	stderr := &outputWriter{ctx: ctx, scope: state, stream: "stderr"}
	env := make([]string, 0, len(options.Env))
	for name, value := range options.Env {
		env = append(env, name+"="+value)
	}
	r, err := interp.New(interp.Dir(options.Dir), interp.Env(expand.ListEnviron(env...)),
		interp.StdIO(options.Stdin, output, stderr), interp.CallHandler(processCall),
		interp.ExecHandlers(state.commands, state.processes), interp.OpenHandler(state.open),
		interp.StatHandler(state.stat), interp.PipeHandler(state.pipe), interp.RetryHandler(retryIO))
	if err != nil {
		return Result{Err: err}
	}
	if options.Login {
		err = login(ctx, r, options)
	}
	if err == nil {
		err = runText(ctx, r, script, "job")
	}
	result := Result{Dir: r.Dir}
	var status interp.ExitStatus
	if err == nil {
		result.Code = new(uint8)
	} else if errors.As(err, &status) {
		code := uint8(status)
		result.Code = &code
	} else {
		result.Err = err
		cancel()
	}
	r.WaitBackground()
	state.outputMu.Lock()
	if state.outputErr != nil {
		result.Err = state.outputErr
		result.Code = nil
	}
	state.outputMu.Unlock()
	if parent.Err() != nil {
		signal := "SIGKILL"
		var requested CancelCause
		if errors.As(context.Cause(parent), &requested) {
			signal = string(requested)
		}
		return Result{Signal: signal, Dir: r.Dir}
	}
	return result
}

func runText(ctx context.Context, r *interp.Runner, text, name string) error {
	file, err := syntax.NewParser(syntax.Variant(syntax.LangBash)).Parse(strings.NewReader(text), name)
	if err != nil {
		return err
	}
	return r.Run(ctx, file)
}

// runSetup runs setup or cleanup statements without firing the job's EXIT
// trap, which belongs to the user's script rather than these API operations.
func runSetup(ctx context.Context, r *interp.Runner, text, name string) error {
	file, err := syntax.NewParser(syntax.Variant(syntax.LangBash)).Parse(strings.NewReader(text), name)
	if err != nil {
		return err
	}
	for _, statement := range file.Stmts {
		if err := r.Run(ctx, statement); err != nil {
			return err
		}
		if r.Exited() {
			break
		}
	}
	return nil
}

// login sources the machine profile and first readable user login profile,
// then restores the job's context and requested directory.
func login(ctx context.Context, r *interp.Runner, options Options) error {
	paths := []string{"/etc/profile"}
	home := options.Env["HOME"]
	for _, name := range []string{".bash_profile", ".bash_login", ".profile"} {
		path := filepath.Join(home, name)
		f, err := openWaiting(ctx, path)
		if ctx.Err() != nil {
			return ctx.Err()
		}
		if err != nil {
			continue
		}
		// Only the open answers whether the profile is readable.
		_ = f.Close()
		paths = append(paths, path)
		break
	}
	for _, path := range paths {
		quoted, err := syntax.Quote(path, syntax.LangBash)
		if err != nil {
			return err
		}
		if err := runSetup(ctx, r, "if [ -r "+quoted+" ]; then . "+quoted+"; fi", "login"); err != nil {
			var status interp.ExitStatus
			if !errors.As(err, &status) || r.Exited() {
				return err
			}
		}
	}
	owned := maps.Clone(options.Env)
	if _, ok := owned["DEMI_CONTEXT_ID"]; ok {
		paths := filepath.SplitList(owned["PATH"])
		if len(paths) == 0 {
			return errors.New("command context has no alias directory")
		}
		alias := paths[0]
		paths = []string{alias}
		for _, path := range filepath.SplitList(r.Vars["PATH"].String()) {
			if path != alias {
				paths = append(paths, path)
			}
		}
		owned["PATH"] = strings.Join(paths, string(os.PathListSeparator))
	}
	for name, value := range owned {
		if strings.HasPrefix(name, "DEMI_") || name == "TMPDIR" || name == "TEMP" || name == "PATH" && owned["DEMI_CONTEXT_ID"] != "" {
			quoted, err := syntax.Quote(name+"="+value, syntax.LangBash)
			if err != nil {
				return err
			}
			if err := runSetup(ctx, r, "export "+quoted, "restore context"); err != nil {
				return err
			}
		}
	}
	quoted, err := syntax.Quote(options.Dir, syntax.LangBash)
	if err != nil {
		return err
	}
	return runSetup(ctx, r, "cd "+quoted, "restore directory")
}

type scope struct {
	options   Options
	outputMu  sync.Mutex
	outputErr error
	cancel    context.CancelFunc
}

type outputWriter struct {
	ctx    context.Context
	scope  *scope
	stream string
}

func (w *outputWriter) Write(p []byte) (int, error) {
	if err := w.ctx.Err(); err != nil {
		return 0, err
	}
	w.scope.outputMu.Lock()
	defer w.scope.outputMu.Unlock()
	if w.scope.options.Output != nil {
		if err := w.scope.options.Output.Write(w.ctx, w.stream, p); err != nil {
			if w.scope.outputErr == nil {
				w.scope.outputErr = err
			}
			w.scope.cancel()
			return 0, err
		}
	}
	return len(p), nil
}

// environment selects exported values, excluding shell-local variables.
func environment(env expand.Environ) map[string]string {
	result := make(map[string]string)
	for name, value := range env.Each {
		if value.Exported && value.IsSet() {
			result[name] = value.String()
		}
	}
	return result
}

func diagnostic(ctx context.Context, name string, err error) error {
	if _, failure := fmt.Fprintf(interp.HandlerCtx(ctx).Stderr, "%s: %v\n", name, err); failure != nil {
		return failure
	}
	return interp.ExitStatus(1)
}

var _ io.Writer = (*outputWriter)(nil)

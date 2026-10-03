package engine

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

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"
	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// Observer exposes shell activity to test scopes.
type Observer interface {
	Check()
	Waiting(int)
}

// Options supplies one interpreter invocation and its job-owned services.
type Options struct {
	Login          bool
	Cwd            string
	Env            map[string]string
	Stdin          *os.File
	Stdout, Stderr io.Writer
	Commands       *process.JobCommands
	Edits          *cmdsdk.Recorder
	Observe        Observer
	// Interrupt closes job-owned pipes after child starts have stopped.
	Interrupt func()
}

// Result preserves the foreground status and final working directory.
type Result struct {
	Code uint8
	Cwd  string
}

// interpreterScope keeps per-subshell process attributes separate from job ownership.
type interpreterScope struct {
	attributes process.ChildAttributes
	owner      *execution
	parent     *interpreterScope
	traps      map[string]string
}

func (s *interpreterScope) Clone() interp.ScopeState {
	clone := &interpreterScope{attributes: s.attributes, owner: s.owner, parent: s, traps: maps.Clone(s.traps)}
	clone.attributes.Limits = append([]process.ResourceLimit(nil), s.attributes.Limits...)
	return clone
}

func (s *interpreterScope) register(command *process.Command) func() {
	s.owner.mu.Lock()
	if s.owner.commands == nil {
		s.owner.commands = make(map[*process.Command]*interpreterScope)
	}
	s.owner.commands[command] = s
	s.owner.mu.Unlock()
	return func() {
		s.owner.mu.Lock()
		delete(s.owner.commands, command)
		s.owner.mu.Unlock()
	}
}

func (s *interpreterScope) Check() {
	if s.owner.options.Observe != nil {
		s.owner.options.Observe.Check()
	}
}
func (s *interpreterScope) Waiting(delta int) {
	if s.owner.options.Observe != nil {
		s.owner.options.Observe.Waiting(delta)
	}
}

type execution struct {
	options      Options
	mu           sync.Mutex // Protects retained redirections; never held during IO.
	files        []io.Closer
	commands     map[*process.Command]*interpreterScope
	starting     int
	stopping     bool
	launchesDone chan struct{}
}

// Execute runs a job's interpreter, retaining redirections until all descendants finish.
func Execute(ctx context.Context, script string, options Options) (result Result, err error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	defer interruptStreams(ctx, options)()
	e := &execution{options: options}
	defer e.interrupt(ctx)()
	defer e.close()
	state := &interpreterScope{owner: e}
	env := make([]string, 0, len(options.Env))
	for name, value := range options.Env {
		env = append(env, name+"="+value)
	}
	r, err := interp.New(interp.Dir(options.Cwd), interp.Env(expand.ListEnviron(env...)), interp.StdIO(options.Stdin, options.Stdout, options.Stderr), interp.ExecHandlers(func(interp.ExecHandlerFunc) interp.ExecHandlerFunc { return e.external }), interp.OpenHandler(e.open), interp.Builtins(e.builtins()), interp.WithScopeState(state), interp.PipeHandler(e.pipe))
	if err != nil {
		return result, err
	}
	defer func() {
		if recovered := recover(); recovered != nil {
			err = errors.New("shell worker panicked")
		}
		if err != nil {
			cancel()
		}
		r.Wait()
	}()
	if options.Login {
		e.profile(ctx, r, "/etc/profile")
		for _, name := range []string{".bash_profile", ".bash_login", ".profile"} {
			if e.profile(ctx, r, filepath.Join(options.Env["HOME"], name)) {
				break
			}
		}
		if err = restoreContext(ctx, r, options.Env, options.Cwd); err != nil {
			return result, err
		}
	}
	node, err := syntax.NewParser().Parse(strings.NewReader(script), "script")
	if err != nil {
		_, _ = fmt.Fprintln(options.Stderr, err)
		return Result{Code: 2, Cwd: r.Dir}, nil
	}
	err = r.Run(ctx, node)
	if ctx.Err() != nil {
		return result, ctx.Err()
	}
	var status interp.ExitStatus
	if errors.As(err, &status) {
		result.Code = uint8(status)
		err = nil
	}
	result.Cwd = r.Dir
	return result, err
}

// profile sources a readable login profile; diagnostics do not discard the job script.
func (e *execution) profile(ctx context.Context, r *interp.Runner, path string) bool {
	file, err := os.Open(path)
	if err != nil {
		return false
	}
	node, parseErr := syntax.NewParser().Parse(file, path)
	closeErr := file.Close()
	err = errors.Join(parseErr, closeErr)
	if err == nil {
		err = r.Run(ctx, node)
	}
	if err != nil {
		_, _ = fmt.Fprintln(e.options.Stderr, err)
	}
	return true
}

// restoreContext restores only runner-owned environment and the requested directory.
func restoreContext(ctx context.Context, r *interp.Runner, env map[string]string, cwd string) error {
	var script strings.Builder
	restored := maps.Clone(env)
	if _, ok := env[process.ContextEnv]; ok {
		paths := filepath.SplitList(env["PATH"])
		if len(paths) == 0 {
			return errors.New("command context has no alias directory")
		}
		configured := r.Vars["PATH"].String()
		kept := []string{paths[0]}
		for _, path := range filepath.SplitList(configured) {
			if path != paths[0] {
				kept = append(kept, path)
			}
		}
		restored["PATH"] = strings.Join(kept, string(os.PathListSeparator))
	}
	for name, value := range restored {
		if strings.HasPrefix(name, "DEMI_") || name == "TMPDIR" || name == "TEMP" || (name == "PATH" && env[process.ContextEnv] != "") {
			quoted, err := syntax.Quote(value, syntax.LangBash)
			if err != nil {
				return err
			}
			fmt.Fprintf(&script, "export %s=%s\n", name, quoted)
		}
	}
	quoted, err := syntax.Quote(cwd, syntax.LangBash)
	if err != nil {
		return err
	}
	fmt.Fprintf(&script, "cd %s\n", quoted)
	node, err := syntax.NewParser().Parse(strings.NewReader(script.String()), "context")
	if err != nil {
		return err
	}
	return r.Run(ctx, node)
}

func (e *execution) close() {
	for _, file := range e.files {
		_ = file.Close()
	} // Interpreter may already have closed the same handle.
}

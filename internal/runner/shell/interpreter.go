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

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

type observation interface {
	check()
	waiting(int)
}

type executionOptions struct {
	login          bool
	cwd            string
	env            map[string]string
	stdin          *os.File
	stdout, stderr io.Writer
	commands       *process.JobCommands
	edits          *cmdsdk.Recorder
	observe        observation
}

type executionResult struct {
	code uint8
	cwd  string
}

// interpreterScope keeps per-subshell process attributes separate from job ownership.
type interpreterScope struct {
	attributes process.ChildAttributes
	owner      *execution
	mu         sync.Mutex // Protects child scopes and currently running commands.
	children   []*interpreterScope
	commands   map[*process.Command]struct{}
}

func (s *interpreterScope) Clone() interp.ScopeState {
	clone := &interpreterScope{attributes: s.attributes, owner: s.owner}
	clone.attributes.Limits = append([]process.ResourceLimit(nil), s.attributes.Limits...)
	s.mu.Lock()
	s.children = append(s.children, clone)
	s.mu.Unlock()
	return clone
}

// signal sends to each process in a background scope without holding a lock over OS calls.
func (s *interpreterScope) signal(signal runnerwire.Signal) error {
	s.mu.Lock()
	children := append([]*interpreterScope(nil), s.children...)
	commands := make([]*process.Command, 0, len(s.commands))
	for command := range s.commands {
		commands = append(commands, command)
	}
	s.mu.Unlock()
	var err error
	for _, command := range commands {
		err = errors.Join(err, command.Signal(signal))
	}
	for _, child := range children {
		err = errors.Join(err, child.signal(signal))
	}
	return err
}

func (s *interpreterScope) register(command *process.Command) func() {
	s.mu.Lock()
	if s.commands == nil {
		s.commands = make(map[*process.Command]struct{})
	}
	s.commands[command] = struct{}{}
	s.mu.Unlock()
	return func() { s.mu.Lock(); delete(s.commands, command); s.mu.Unlock() }
}

func (s *interpreterScope) Check() {
	if s.owner.options.observe != nil {
		s.owner.options.observe.check()
	}
}
func (s *interpreterScope) Waiting(delta int) {
	if s.owner.options.observe != nil {
		s.owner.options.observe.waiting(delta)
	}
}

type execution struct {
	options executionOptions
	mu      sync.Mutex // Protects retained redirections; never held during IO.
	files   []io.Closer
}

// execute runs a job's interpreter, retaining redirections until all descendants finish.
func execute(ctx context.Context, script string, options executionOptions) (result executionResult, err error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	e := &execution{options: options}
	defer e.close()
	state := &interpreterScope{owner: e}
	env := make([]string, 0, len(options.env))
	for name, value := range options.env {
		env = append(env, name+"="+value)
	}
	r, err := interp.New(interp.Dir(options.cwd), interp.Env(expand.ListEnviron(env...)), interp.StdIO(options.stdin, options.stdout, options.stderr), interp.ExecHandler(e.external), interp.OpenHandler(e.open), interp.Builtins(e.builtins()), interp.WithScopeState(state))
	if err != nil {
		return result, err
	}
	defer func() {
		if recovered := recover(); recovered != nil {
			err = fmt.Errorf("shell worker panicked: %v", recovered)
		}
		if err != nil {
			cancel()
		}
		r.Wait()
	}()
	if options.login {
		e.profile(ctx, r, "/etc/profile")
		for _, name := range []string{".bash_profile", ".bash_login", ".profile"} {
			if e.profile(ctx, r, filepath.Join(options.env["HOME"], name)) {
				break
			}
		}
		if err = restoreContext(ctx, r, options.env, options.cwd); err != nil {
			return result, err
		}
	}
	node, err := syntax.NewParser().Parse(strings.NewReader(script), "script")
	if err != nil {
		return result, err
	}
	err = r.Run(ctx, node)
	if status, ok := interp.IsExitStatus(err); ok {
		result.code = status
		err = nil
	}
	result.cwd = r.Dir
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
		_, _ = fmt.Fprintln(e.options.stderr, err)
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
	fmt.Fprintf(&script, "cd -- %s\n", quoted)
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

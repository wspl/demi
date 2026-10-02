package shelltest

import (
	"context"
	"errors"
	"os"

	"github.com/wspl/demi/internal/runner/shell/internal/engine"
)

// Options supplies an interpreter execution in an owned test scope.
// Login enables the same profile loading and context restoration as a job.
// The caller owns the files and keeps them open until Execute and Scope.Finish
// have returned. Env replaces the inherited environment.
type Options struct {
	Scope  *Scope
	Login  bool
	Cwd    string
	Env    map[string]string
	Stdin  *os.File
	Stdout *os.File
	Stderr *os.File
}

// Result preserves the foreground exit status and final working directory.
type Result struct {
	Code uint8
	Cwd  string
}

// Execute runs a script and joins its nested interpreter work before returning.
// A nonzero shell exit is returned in Result; setup and interpreter failures
// return an error. Context cancellation stops and joins the execution.
func Execute(ctx context.Context, script string, options Options) (Result, error) {
	if options.Scope == nil {
		return Result{}, errors.New("missing shell execution owner")
	}
	ctx, finish := options.Scope.execution(ctx)
	defer finish()
	result, err := engine.Execute(ctx, script, engine.Options{Login: options.Login, Cwd: options.Cwd, Env: options.Env, Stdin: options.Stdin, Stdout: options.Stdout, Stderr: options.Stderr, Commands: options.Scope.Commands, Edits: options.Scope.Edits, Observe: observer{options.Scope.Activity()}})
	return Result{Code: result.Code, Cwd: result.Cwd}, err
}

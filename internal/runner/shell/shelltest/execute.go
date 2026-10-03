package shelltest

import (
	"context"
	"errors"
	"os"

	"github.com/wspl/demi/internal/runner/shell/internal/engine"
)

// Options supplies an interpreter execution in an owned test scope.
// The caller owns the files and keeps them open until Execute and Scope.Finish
// have returned.
type Options struct {
	// Scope owns the execution and its cancellation callbacks.
	Scope *Scope
	// Login enables profile loading and restoration of the owned context.
	Login bool
	// Cwd is the directory requested for the script.
	Cwd string
	// Env replaces the inherited environment.
	Env map[string]string
	// Stdin is borrowed until execution and scope cleanup have returned.
	Stdin *os.File
	// Stdout receives script output through a caller-owned file.
	Stdout *os.File
	// Stderr receives diagnostics through a caller-owned file.
	Stderr *os.File
}

// Result preserves the foreground exit status and final working directory.
type Result struct {
	// Code excludes statuses of nested background work.
	Code uint8
	// Cwd is the directory when the foreground script finishes.
	Cwd string
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
	result, err := engine.Execute(
		ctx,
		script,
		engine.Options{
			Login:    options.Login,
			Cwd:      options.Cwd,
			Env:      options.Env,
			Stdin:    options.Stdin,
			Stdout:   options.Stdout,
			Stderr:   options.Stderr,
			Commands: options.Scope.Commands,
			Edits:    options.Scope.Edits,
			Observe:  observer{options.Scope.Activity()},
		},
	)
	return Result{Code: result.Code, Cwd: result.Cwd}, err
}

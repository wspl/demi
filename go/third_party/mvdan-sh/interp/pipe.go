package interp

import (
	"context"
	"os"
)

// PipeHandlerFunc creates an interpreter pipe. A host can wait out descriptor
// exhaustion here, with the same context that owns the shell operation.
type PipeHandlerFunc func(context.Context) (*os.File, *os.File, error)

// PipeHandler replaces pipe creation for pipelines and here documents/strings.
func PipeHandler(handler PipeHandlerFunc) RunnerOption {
	return func(r *Runner) error { r.pipeHandler = handler; return nil }
}
func (r *Runner) pipe(ctx context.Context) (stdinFile, *os.File, error) {
	if r.pipeHandler != nil {
		return r.pipeHandler(ctx)
	}
	return newPipe()
}

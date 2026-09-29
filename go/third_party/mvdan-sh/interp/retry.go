package interp

import "context"

// RetryHandlerFunc runs an interpreter I/O allocation, optionally retrying
// transient failures. Each attempt must release resources on failure. The
// handler must honor ctx; cleanup operations may use a noncancelable context.
type RetryHandlerFunc func(ctx context.Context, attempt func() error) error

// RetryHandler supplies the Host's allocation retry policy for file opens,
// directory enumeration, and internal cancellation and descriptor pipes.
func RetryHandler(handler RetryHandlerFunc) RunnerOption {
	return func(r *Runner) error {
		r.retryHandler = handler
		return nil
	}
}

type retryKey struct{}

func retryIO(ctx context.Context, attempt func() error) error {
	if handler, ok := ctx.Value(retryKey{}).(RetryHandlerFunc); ok && handler != nil {
		return handler(ctx, attempt)
	}
	return attempt()
}

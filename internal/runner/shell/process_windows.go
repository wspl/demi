package shell

import (
	"context"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) umask(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}
func (e *execution) ulimit(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}
func (e *execution) kill(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}

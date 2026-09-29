package shell

import "context"

func kill(ctx context.Context, args []string) error { return runProcess(ctx, args) }

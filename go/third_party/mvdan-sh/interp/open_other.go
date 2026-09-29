//go:build !unix

package interp

import (
	"context"
	"os"
)

func openContext(ctx context.Context, path string, flags int, mode os.FileMode) (*os.File, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return openFile(ctx, path, flags, mode)
}

package engine

import (
	"context"
	"io"
	"os/exec"
)

// extraDescriptors leaves Windows descriptor inheritance to standard IO.
func extraDescriptors(_ context.Context, _ <-chan struct{}, _ *exec.Cmd, _ map[string]io.ReadWriteCloser) (func() error, error) {
	return func() error { return nil }, nil
}

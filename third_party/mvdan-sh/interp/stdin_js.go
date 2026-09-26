// Copyright (c) 2017, Daniel Martí <mvdan@mvdan.cc>
// See LICENSE for licensing information

//go:build js

package interp

import (
	"context"
	"io"
)

// DefaultPipeHandler returns the [PipeHandlerFunc] used by default.
// js/wasm has no OS pipes and no subprocesses, so it creates an [io.Pipe].
func DefaultPipeHandler() PipeHandlerFunc {
	return func(ctx context.Context) (io.ReadCloser, io.WriteCloser, error) {
		pr, pw := io.Pipe()
		return pr, pw, nil
	}
}

// stdinTerminal always reports false, as js/wasm has no terminals.
func stdinTerminal(stdin io.Reader) (int, bool) { return -1, false }

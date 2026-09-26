// Copyright (c) 2017, Daniel Martí <mvdan@mvdan.cc>
// See LICENSE for licensing information

//go:build !js

package interp

import (
	"context"
	"io"
	"os"

	"golang.org/x/term"
)

// DefaultPipeHandler returns the [PipeHandlerFunc] used by default.
// It creates an [os.Pipe], whose ends subprocesses can inherit.
func DefaultPipeHandler() PipeHandlerFunc {
	return func(ctx context.Context) (io.ReadCloser, io.WriteCloser, error) {
		return os.Pipe()
	}
}

// stdinTerminal returns the file descriptor of the shell's stdin
// if it is a terminal.
// Note that we only call [os.File.Fd] on character devices,
// as it stops [os.File.SetReadDeadline] from working,
// which [Runner.readLine] needs to cancel blocking reads.
func stdinTerminal(stdin io.Reader) (int, bool) {
	file, ok := stdin.(*os.File)
	if !ok {
		return -1, false
	}
	fi, err := file.Stat()
	if err != nil || fi.Mode()&os.ModeCharDevice == 0 {
		return -1, false
	}
	fd := int(file.Fd())
	return fd, term.IsTerminal(fd)
}

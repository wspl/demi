package cli

import (
	"bufio"
	"context"
	"fmt"
	"io"

	"github.com/wspl/demi/internal/toolctx"
)

// StdinName is the operand that names standard input.
const StdinName = "-"

// Input is an operand opened for reading.
type Input struct {
	io.Reader
	// File is the opened file, or nil for standard input.
	File toolctx.File
}

// Open opens an input operand; "-" is standard input. Reads stop with the
// context's error once the job is cancelled.
func Open(inv *toolctx.Invocation, name string) (*Input, error) {
	if name == StdinName {
		return &Input{Reader: Reader(inv.Context, inv.Stdin)}, nil
	}
	file, err := inv.Files.Open(name)
	if err != nil {
		return nil, err
	}
	return &Input{Reader: Reader(inv.Context, file), File: file}, nil
}

// Close closes the opened file; standard input stays open.
func (in *Input) Close() error {
	if in.File == nil {
		return nil
	}
	return in.File.Close()
}

// Release closes an input that is given up after an error. It was only
// read, so a failed close loses nothing and is not reported.
func (in *Input) Release() {
	_ = in.Close()
}

// Reader returns r that fails with ctx's error once ctx is done, so a loop
// that reads it stops at cancellation.
func Reader(ctx context.Context, r io.Reader) io.Reader {
	return &contextReader{ctx: ctx, r: r}
}

type contextReader struct {
	ctx context.Context
	r   io.Reader
}

func (c *contextReader) Read(p []byte) (int, error) {
	if err := c.ctx.Err(); err != nil {
		return 0, err
	}
	return c.r.Read(p)
}

// Writer returns w that fails with ctx's error once ctx is done.
func Writer(ctx context.Context, w io.Writer) io.Writer {
	return &contextWriter{ctx: ctx, w: w}
}

type contextWriter struct {
	ctx context.Context
	w   io.Writer
}

func (c *contextWriter) Write(p []byte) (int, error) {
	if err := c.ctx.Err(); err != nil {
		return 0, err
	}
	return c.w.Write(p)
}

// Output buffers what a utility writes. bufio.Writer keeps its first write
// error and returns it from every later call, so the Put methods drop the
// per-call result; a loop checks Err to stop early, and Flush reports the
// error at the end.
type Output struct {
	w *bufio.Writer
}

// NewOutput buffers w.
func NewOutput(w io.Writer) *Output {
	return &Output{w: bufio.NewWriterSize(w, 64*1024)}
}

// Put writes p.
func (o *Output) Put(p []byte) {
	_, _ = o.w.Write(p)
}

// PutByte writes c.
func (o *Output) PutByte(c byte) {
	_ = o.w.WriteByte(c)
}

// PutString writes s.
func (o *Output) PutString(s string) {
	_, _ = o.w.WriteString(s)
}

// Putf writes formatted text.
func (o *Output) Putf(format string, args ...any) {
	_, _ = fmt.Fprintf(o.w, format, args...)
}

// Err returns the first write error, if any.
func (o *Output) Err() error {
	_, err := o.w.Write(nil)
	return err
}

// Flush writes the buffer out and returns the first write error.
func (o *Output) Flush() error {
	return o.w.Flush()
}

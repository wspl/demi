package cmdsdk

import (
	"context"
	"encoding/binary"
	"errors"
	"io"

	"github.com/wspl/demi/internal/commandwire"
)

// InputSource supplies one bounded invocation input chunk; io.EOF ends input.
type InputSource interface {
	Next(context.Context) ([]byte, error)
}

// OutputSink receives the two invocation output channels.
type OutputSink interface {
	Stdout(context.Context, []byte) error
	Stderr(context.Context, []byte) error
}

// Input is the handler's demand-driven input.
type Input struct{ source InputSource }

// NewInput adapts a local source without buffering or reading ahead.
func NewInput(source InputSource) *Input { return &Input{source: source} }

// Next requests one chunk, preserving its boundary.
func (i *Input) Next(ctx context.Context) ([]byte, error) { return i.source.Next(ctx) }

// Output shares an invocation's bounded output queue. It is safe for concurrent writers.
type Output struct {
	ctx     context.Context
	records chan commandwire.Record
}

// OutputChannel creates the four-record queue used by local and remote handlers.
func OutputChannel(ctx context.Context) (*Output, <-chan commandwire.Record) {
	o := &Output{ctx: ctx, records: make(chan commandwire.Record, 4)}
	return o, o.records
}
func (o *Output) send(ctx context.Context, r commandwire.Record) error {
	if err := o.ctx.Err(); err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-o.ctx.Done():
		return o.ctx.Err()
	case o.records <- r:
		return nil
	}
}

// Stdout writes bytes as bounded standard output records.
func (o *Output) Stdout(ctx context.Context, b []byte) error { return o.write(ctx, b, false) }

// Stderr writes bytes as bounded standard error records.
func (o *Output) Stderr(ctx context.Context, b []byte) error { return o.write(ctx, b, true) }
func (o *Output) write(ctx context.Context, b []byte, stderr bool) error {
	for len(b) > 0 {
		n := min(len(b), commandwire.MaxRecordBytes)
		chunk := append([]byte(nil), b[:n]...)
		var r commandwire.Record = commandwire.Stdout(chunk)
		if stderr {
			r = commandwire.Stderr(chunk)
		}
		if err := o.send(ctx, r); err != nil {
			return err
		}
		b = b[n:]
	}
	return nil
}

type httpInput struct {
	body   io.Reader
	output *Output
	ended  bool
}

func (i *httpInput) Next(ctx context.Context) ([]byte, error) {
	if i.ended {
		return nil, io.EOF
	}
	if err := i.output.send(ctx, commandwire.InputPull{}); err != nil {
		return nil, err
	}
	stop := context.AfterFunc(ctx, func() {
		if c, ok := i.body.(io.Closer); ok {
			_ = c.Close()
		}
	})
	defer stop()
	b, err := readChunk(i.body, commandwire.MaxRecordBytes)
	if errors.Is(err, io.EOF) {
		i.ended = true
	}
	return b, err
}

// readChunk reads one command length prefix before allocating its bounded payload.
func readChunk(r io.Reader, limit uint32) ([]byte, error) {
	var prefix [4]byte
	if _, err := io.ReadFull(r, prefix[:]); err != nil {
		return nil, err
	}
	n := binary.BigEndian.Uint32(prefix[:])
	if n > limit {
		return nil, commandwire.ErrTooLarge
	}
	b := make([]byte, n)
	_, err := io.ReadFull(r, b)
	return b, err
}

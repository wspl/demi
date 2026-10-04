package commandsdk

import (
	"context"
	"encoding/binary"
	"errors"
	"io"

	"github.com/wspl/demi/internal/commandproto"
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
	records chan commandproto.Record
}

// OutputChannel creates the four-record queue used by local and remote handlers.
func OutputChannel(ctx context.Context) (*Output, <-chan commandproto.Record) {
	o := &Output{ctx: ctx, records: make(chan commandproto.Record, 4)}
	return o, o.records
}

func (o *Output) send(ctx context.Context, r commandproto.Record) error {
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
		n := min(len(b), commandproto.MaxRecordBytes)
		chunk := append([]byte(nil), b[:n]...)
		var r commandproto.Record = commandproto.Stdout(chunk)
		if stderr {
			r = commandproto.Stderr(chunk)
		}
		if err := o.send(ctx, r); err != nil {
			return err
		}
		b = b[n:]
	}
	return nil
}

type httpInput struct {
	body      io.Reader
	output    *Output
	remaining int64 // -1 when the request has no Content-Length.
	ended     bool
}

// Next requests and reads one bounded invocation input chunk.
func (i *httpInput) Next(ctx context.Context) ([]byte, error) {
	if i.ended {
		return nil, io.EOF
	}
	// A finite request needs no further pull once its declared bytes have
	// been consumed; still read to verify END_STREAM rather than assuming EOF.
	// Streaming requests report EOF in answer to a pull.
	if i.remaining != 0 {
		if err := i.output.send(ctx, commandproto.InputPull{}); err != nil {
			return nil, err
		}
	}
	stop := context.AfterFunc(ctx, func() {
		if c, ok := i.body.(io.Closer); ok {
			_ = c.Close()
		}
	})
	defer stop()
	b, err := readChunk(i.body, commandproto.MaxRecordBytes)
	if err == nil && i.remaining >= 0 {
		i.remaining -= int64(4 + len(b))
	}
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
		return nil, commandproto.ErrTooLarge
	}
	b := make([]byte, n)
	_, err := io.ReadFull(r, b)
	return b, err
}

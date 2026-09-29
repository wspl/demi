package commandservice

import (
	"context"
	"io"
	"slices"
)

// A Handler serves the operations of a command program. [Serve] calls it from
// one goroutine per invocation, so it must be safe for concurrent use.
// Handlers own per-invocation state and must not change the process-wide
// working directory or environment for an invocation.
type Handler interface {
	// Operations returns the operations the handler serves, each once. It is
	// the service's catalog, which the runner checks against the package's
	// descriptor.
	Operations() []string
	// Invoke runs one invocation and returns how it ended. An error becomes a
	// completion with exit code 1 and the error command_failed, whose message
	// is the error's text (bytes of it that are not valid UTF-8 become U+FFFD),
	// unless the call was cancelled, when nothing more is sent. A handler must
	// return soon after its call's context ends: one that does not is a fault
	// of the service.
	Invoke(call *Call) (Completion, error)
}

// A ConversationHandler is a [Handler] that holds conversation state and
// answers the conversation endpoint itself. Without it a service holds none:
// status answers {"conversations":[]} and release answers {}, both with exit
// code 0.
//
// A release must end everything the handler holds for its conversation and
// return once that is cleaned up. One that fails, or ends with a nonzero exit
// code, is a fault of the service.
type ConversationHandler interface {
	Conversation(call *ConversationCall) (Completion, error)
}

// A Closer is a [Handler] that has something to release when its service
// stops. Close is called once, after every call has stopped and before [Serve]
// returns, with a deadline of the cancellation grace.
type Closer interface {
	Close(ctx context.Context) error
}

// A Call is one invocation, as its handler sees it. It is scoped to the
// invocation, like an [net/http.Request], and its input and output are bound
// to it.
type Call struct {
	Invocation Invocation
	// Stdin is the invocation's input.
	Stdin *Input
	// Stdout and Stderr queue the invocation's output in records of at most
	// [MaxRecordBytes]. A write blocks while the queue is full, and fails with
	// [ErrCancelled] once the invocation is cancelled.
	Stdout io.Writer
	Stderr io.Writer
	// Numbers is the connection's numbers source.
	Numbers *Numbers

	state *callState
}

// Context returns the context of the invocation. It ends when the caller
// cancels the invocation, when the service stops, and when the call finishes.
func (c *Call) Context() context.Context { return c.state.ctx }

// A ConversationCall is one request to the conversation endpoint. The endpoint
// takes no input and carries no environment.
type ConversationCall struct {
	Request ConversationRequest
	// Stdout queues the answer, as [Call.Stdout] does.
	Stdout io.Writer

	state *callState
}

// Context returns the context of the request, which ends as a [Call]'s does.
func (c *ConversationCall) Context() context.Context { return c.state.ctx }

// A callState is what one call reads and writes, bound to the call's context,
// which it alone holds: the request body its input arrives through, one chunk
// for each pull, and the queue its output leaves through. The queue holds at
// most [outputQueueRecords]: a handler that outruns its caller blocks.
type callState struct {
	ctx     context.Context
	body    io.ReadCloser
	records chan Record
}

func newCallState(ctx context.Context, body io.ReadCloser) *callState {
	return &callState{ctx: ctx, body: body, records: make(chan Record, outputQueueRecords)}
}

// send queues record, or fails with [ErrCancelled] once the call is cancelled.
func (c *callState) send(record Record) error {
	if c.ctx.Err() != nil {
		return ErrCancelled
	}
	select {
	case c.records <- record:
		return nil
	case <-c.ctx.Done():
		return ErrCancelled
	}
}

// chunk asks the caller for one input chunk, by queueing an input pull, and
// reads it. It returns [io.EOF] when the caller ended its input.
func (c *callState) chunk() ([]byte, error) {
	if err := c.send(Record{Kind: RecordInputPull}); err != nil {
		return nil, err
	}
	chunk, err := readFrame(c.body, MaxRecordBytes)
	if err != nil {
		if c.ctx.Err() != nil {
			return nil, ErrCancelled
		}
		return nil, err
	}
	return chunk, nil
}

// abort wakes a read of the input that is blocked, which then fails.
func (c *callState) abort() {
	// The body's close error says only that it was closed before.
	_ = c.body.Close()
}

// writer returns a writer that queues what it is given as records of kind.
func (c *callState) writer(kind RecordKind) io.Writer {
	return recordWriter{state: c, kind: kind}
}

type recordWriter struct {
	state *callState
	kind  RecordKind
}

func (w recordWriter) Write(p []byte) (int, error) {
	written := 0
	for len(p) > 0 {
		count := min(len(p), MaxRecordBytes)
		// The queue keeps the record after Write returns, so it gets a copy.
		if err := w.state.send(Record{Kind: w.kind, Data: slices.Clone(p[:count])}); err != nil {
			return written, err
		}
		written += count
		p = p[count:]
	}
	return written, nil
}

// An Input is the input of an invocation. The handler pulls it one chunk at a
// time: each call to Next asks the caller for exactly one chunk, or its end.
// An Input is not safe for concurrent use.
type Input struct {
	state *callState
	err   error
}

// Next returns the next chunk of input, a slice of at most [MaxRecordBytes]
// that the caller of Next owns, or [io.EOF] when the caller ended its input.
// It returns [ErrCancelled] once the invocation is cancelled.
func (in *Input) Next() ([]byte, error) {
	if in.err != nil {
		return nil, in.err
	}
	chunk, err := in.state.chunk()
	if err != nil {
		in.err = err
		return nil, err
	}
	return chunk, nil
}

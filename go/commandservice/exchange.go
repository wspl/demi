package commandservice

import (
	"context"
	"fmt"
	"io"
	"sync"
)

// A Side says which side of an exchange failed.
type Side int

// The sides of an exchange.
const (
	// ServiceSide is the service, or the connection to it: it failed, or it
	// broke the protocol.
	ServiceSide Side = iota + 1
	// InputSide is the caller's source of input.
	InputSide
	// OutputSide is the caller's standard output or standard error.
	OutputSide
)

// An ExchangeError is the failure of an [Exchange], and says which side failed.
type ExchangeError struct {
	Side Side
	Err  error
}

func (e *ExchangeError) Error() string {
	switch e.Side {
	case InputSide:
		return fmt.Sprintf("reading the invocation's input: %v", e.Err)
	case OutputSide:
		return fmt.Sprintf("writing the invocation's output: %v", e.Err)
	default:
		return fmt.Sprintf("the service: %v", e.Err)
	}
}

func (e *ExchangeError) Unwrap() error { return e.Err }

// Exchange runs an invocation, which stream carries, to its completion: each
// input pull of the service is answered with one Read of input, up to
// [MaxRecordBytes] (a short read is one chunk, and the end of input ends the
// request stream), and the service's records go to stdout and stderr. The end
// of input ends the input only; the invocation runs on until it completes.
//
// Any failure cancels the stream. The error is an [*ExchangeError] that says
// which side failed, or the error of ctx when it ended the exchange. A Read of
// input that is waiting when the invocation completes is left to return on its
// own.
func Exchange(ctx context.Context, stream *Stream, input io.Reader, stdout, stderr io.Writer) (Completion, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	e := &exchange{stream: stream}
	stopWatching := context.AfterFunc(ctx, func() { e.fail(ctx.Err()) })
	defer stopWatching()

	pulls := make(chan struct{}, 1)
	go e.answerPulls(ctx, input, pulls)

	var completion Completion
	completed := false
	for {
		record, err := stream.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			e.fail(&ExchangeError{Side: ServiceSide, Err: err})
			break
		}
		switch record.Kind {
		case RecordStdout:
			if _, err := stdout.Write(record.Data); err != nil {
				e.fail(&ExchangeError{Side: OutputSide, Err: err})
			}
		case RecordStderr:
			if _, err := stderr.Write(record.Data); err != nil {
				e.fail(&ExchangeError{Side: OutputSide, Err: err})
			}
		case RecordInputPull:
			select {
			case pulls <- struct{}{}:
			default:
				e.fail(&ExchangeError{Side: ServiceSide, Err: &InvalidError{Reason: "overlapping input demands"}})
			}
		case RecordCompletion:
			completion = record.Completion
			completed = true
		}
		if e.failure() != nil {
			break
		}
	}
	if err := e.failure(); err != nil {
		return Completion{}, err
	}
	if !completed {
		return Completion{}, &ExchangeError{Side: ServiceSide, Err: ErrIncomplete}
	}
	return completion, nil
}

// An exchange holds the first failure of one [Exchange], which any of its
// goroutines may report.
type exchange struct {
	stream *Stream

	mu  sync.Mutex
	err error
}

// fail records the exchange's first failure and cancels the stream, which ends
// the read of its records.
func (e *exchange) fail(err error) {
	e.mu.Lock()
	first := e.err == nil
	if first {
		e.err = err
	}
	e.mu.Unlock()
	if first {
		e.stream.Cancel()
	}
}

func (e *exchange) failure() error {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.err
}

// answerPulls answers each input pull of the service with one chunk of input,
// or with the end of the input.
func (e *exchange) answerPulls(ctx context.Context, input io.Reader, pulls <-chan struct{}) {
	var buffer []byte
	for {
		select {
		case <-pulls:
		case <-ctx.Done():
			return
		}
		if buffer == nil {
			buffer = make([]byte, MaxRecordBytes)
		}
		n, err := io.ReadAtLeast(input, buffer, 1)
		if n > 0 {
			if err := e.stream.Write(buffer[:n]); err != nil {
				e.fail(&ExchangeError{Side: ServiceSide, Err: err})
				return
			}
			continue
		}
		if err != io.EOF {
			e.fail(&ExchangeError{Side: InputSide, Err: err})
			return
		}
		if err := e.stream.End(); err != nil {
			e.fail(&ExchangeError{Side: ServiceSide, Err: err})
		}
		// The completion, not the input, ends the exchange.
		return
	}
}

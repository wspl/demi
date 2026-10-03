package provider

import (
	"context"
	"errors"
	"io"
	"iter"
	"strings"
	"sync"
	"unicode/utf8"

	sse "github.com/tmaxmax/go-sse"
	"github.com/wspl/demi/internal/contract"
)

// SSEErrorKind distinguishes a broken body from invalid text or framing.
type SSEErrorKind uint8

// Event stream failure categories.
const (
	// SSETransport identifies a body transport failure.
	SSETransport SSEErrorKind = iota
	// SSEUTF8 identifies an invalid UTF-8 body.
	SSEUTF8
	// SSESyntax identifies an invalid event-stream frame.
	SSESyntax
)

// SSEError explains why an event stream could not be read.
type SSEError struct {
	Kind SSEErrorKind
	Err  error
}

// Error returns the diagnostic for this failure.
func (e *SSEError) Error() string {
	switch e.Kind {
	case SSETransport:
		return "the event stream broke off: " + e.Err.Error()
	case SSEUTF8:
		return "the event stream is not UTF-8: " + e.Err.Error()
	default:
		return "the event stream cannot be parsed: " + e.Err.Error()
	}
}

// Unwrap returns the underlying cause.
func (e *SSEError) Unwrap() error { return e.Err }

var sseConfig = sse.ReadConfig{MaxEventSize: int(^uint(0) >> 1)}

// SSEData yields joined data fields, ignoring empty events. It retains a final
// unterminated frame at clean EOF and validates UTF-8 across reads. Iteration
// owns body: cancellation, exhaustion and early exit all close it.
func SSEData(ctx context.Context, body io.ReadCloser) iter.Seq2[string, error] {
	return func(yield func(string, error) bool) {
		stop := closeOnCancel(ctx, body)
		defer stop()
		checked := &utf8Body{source: io.MultiReader(body, strings.NewReader("\n\n"))}
		for event, err := range sse.Read(checked, &sseConfig) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				if checked.end != nil && !errors.Is(checked.end, io.EOF) {
					err = checked.end
				}
				var stream *SSEError
				if !errors.As(err, &stream) {
					err = &SSEError{Kind: SSESyntax, Err: err}
				}
				yield("", err)
				return
			}
			if event.Data != "" && !yield(event.Data, nil) {
				return
			}
		}
	}
}

// closeOnCancel owns a response body and joins its cancellation callback.
func closeOnCancel(ctx context.Context, body io.Closer) func() {
	var once sync.Once
	closeBody := func() { once.Do(func() { _ = body.Close() }) } // The reader reports IO failures; close only releases it.
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(done)
		closeBody()
	})
	return func() {
		if !stop() {
			<-done
		}
		closeBody()
	}
}

// utf8Body retains a non-UTF-8 suffix until the next read, as eventsource-stream does.
type utf8Body struct {
	source  io.Reader
	pending []byte
	ready   []byte
	end     error
}

// Read returns valid UTF-8 while retaining an incomplete suffix.
func (r *utf8Body) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	for len(r.ready) == 0 && r.end == nil {
		buffer := make([]byte, 32*1024)
		n, err := r.source.Read(buffer)
		data := append(r.pending, buffer[:n]...)
		r.pending = nil
		end := len(data)
		for i := 0; i < len(data); {
			ch, width := utf8.DecodeRune(data[i:])
			if ch == utf8.RuneError && width == 1 {
				r.pending = append([]byte(nil), data[i:]...)
				end = i
				break
			}
			i += width
		}
		r.ready = data[:end]
		r.readEnd(err)
	}
	if len(r.ready) > 0 {
		n := copy(p, r.ready)
		r.ready = r.ready[n:]
		return n, nil
	}
	return 0, r.end
}

// vendorInvalidUTF8Width adapts the shared diagnostic for Rust's lossy decoding.
// Its caller passes a suffix beginning with a malformed sequence.
func vendorInvalidUTF8Width(data []byte) int {
	var invalid *contract.UTF8Error
	if errors.As(contract.CheckUTF8(data), &invalid) && invalid.ErrorLen != 0 {
		return invalid.ErrorLen
	}
	return len(data)
}

func (r *utf8Body) readEnd(err error) {
	if err == nil {
		return
	}
	if !errors.Is(err, io.EOF) {
		r.end = &SSEError{Kind: SSETransport, Err: err}
		return
	}
	if len(r.pending) != 0 {
		r.end = &SSEError{Kind: SSEUTF8, Err: contract.CheckUTF8(r.pending)}
		return
	}
	r.end = io.EOF
}

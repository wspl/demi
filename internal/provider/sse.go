package provider

import (
	"context"
	"errors"
	"fmt"
	"io"
	"iter"
	"strings"
	"sync"
	"unicode/utf8"

	sse "github.com/tmaxmax/go-sse"
)

// SSEErrorKind distinguishes a broken body from invalid text or framing.
type SSEErrorKind uint8

// Event stream failure categories.
const (
	SSETransport SSEErrorKind = iota
	SSEUTF8
	SSESyntax
)

// SSEError explains why an event stream could not be read.
type SSEError struct {
	Kind SSEErrorKind
	Err  error
}

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
		if err != nil {
			if errors.Is(err, io.EOF) {
				if len(r.pending) != 0 {
					r.end = &SSEError{Kind: SSEUTF8, Err: invalidSSEUTF8(r.pending)}
				} else {
					r.end = io.EOF
				}
			} else {
				r.end = &SSEError{Kind: SSETransport, Err: err}
			}
		}
	}
	if len(r.ready) > 0 {
		n := copy(p, r.ready)
		r.ready = r.ready[n:]
		return n, nil
	}
	return 0, r.end
}

// invalidSSEUTF8 reports the retained suffix in Rust's Utf8Error format.
func invalidSSEUTF8(data []byte) error {
	if !utf8.FullRune(data) {
		return errors.New("incomplete utf-8 byte sequence from index 0")
	}
	return fmt.Errorf("invalid utf-8 sequence of %d bytes from index 0", vendorInvalidUTF8Width(data))
}

// vendorInvalidUTF8Width counts one malformed sequence as Rust's UTF-8 reader does.
func vendorInvalidUTF8Width(data []byte) int {
	if !utf8.FullRune(data) {
		return len(data)
	}
	size := 1
	lead := data[0]
	if lead >= 0xc2 && lead <= 0xf4 && len(data) > 1 {
		second := data[1]
		valid := second >= 0x80 && second <= 0xbf
		if lead == 0xe0 {
			valid = second >= 0xa0 && second <= 0xbf
		}
		if lead == 0xed {
			valid = second >= 0x80 && second <= 0x9f
		}
		if lead == 0xf0 {
			valid = second >= 0x90 && second <= 0xbf
		}
		if lead == 0xf4 {
			valid = second >= 0x80 && second <= 0x8f
		}
		if valid {
			size = 2
			if lead >= 0xf0 && len(data) > 2 && data[2] >= 0x80 && data[2] <= 0xbf {
				size = 3
			}
		}
	}
	return size
}

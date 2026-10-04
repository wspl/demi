package provider_test

import (
	"bytes"
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
)

// frames reads chunks exactly as split by the scripted body.
func frames(t *testing.T, chunks ...[]byte) []string {
	t.Helper()
	readers := make([]io.Reader, len(chunks))
	for i, chunk := range chunks {
		readers[i] = bytes.NewReader(chunk)
	}
	out := make([]string, 0)
	for data, err := range provider.SSEData(t.Context(), io.NopCloser(io.MultiReader(readers...))) {
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, data)
	}
	return out
}

func TestSSESpecification(t *testing.T) {
	for _, tc := range []struct {
		body string
		want []string
	}{
		{"event: delta\r\ndata: {\"a\":1}\r\n\r\n", []string{`{"a":1}`}},
		{"data: line one\ndata: line two\n\n", []string{"line one\nline two"}},
		{"data: {\"a\":1}\n\ndata: [DONE]\n\n", []string{`{"a":1}`, "[DONE]"}},
		{"", []string{}},
		{"\ufeffdata: x\r\rdata:  y\r\r", []string{"x", " y"}},
	} {
		requireEqual(t, frames(t, []byte(tc.body)), tc.want)
	}
}

func TestSSEFinalFrame(t *testing.T) {
	for _, body := range []string{"data: {\"a\":1}", "data: {\"a\":1}\n"} {
		requireEqual(t, frames(t, []byte(body)), []string{`{"a":1}`})
	}
}

func TestSSESplitUTF8(t *testing.T) {
	text := []byte("data: héllo\n\n")
	requireEqual(t, frames(t, text[:8], text[8:]), []string{"héllo"})
	requireEqual(t, frames(t, []byte("da"), []byte("ta: x"), []byte("\n"), []byte("\n")), []string{"x"})
	for i := 1; i < len(text); i++ {
		requireEqual(t, frames(t, text[:i], text[i:]), []string{"héllo"})
	}
}

func TestSSEEmptyData(t *testing.T) {
	requireEqual(t, frames(t, []byte(": keep-alive\n\nevent: ping\n\ndata:\n\ndata: x\n\n")), []string{"x"})
}

type failedReader struct{ err error }

func (r failedReader) Read([]byte) (int, error) {
	return 0, r.err
}

func TestSSETransportAfterFrames(t *testing.T) {
	broken := errors.New("connection reset")
	body := io.NopCloser(io.MultiReader(strings.NewReader("data: first\n\n"), failedReader{broken}))
	var data []string
	var failure error
	index := 0
	for frame, err := range provider.SSEData(t.Context(), body) {
		if err != nil {
			requireEqual(t, index, 1)
			failure = err
		} else {
			requireEqual(t, index, 0)
			data = append(data, frame)
		}
		index++
	}
	requireEqual(t, data, []string{"first"})
	var stream *provider.SSEError
	if !errors.Is(failure, broken) || !errors.As(failure, &stream) || stream.Kind != provider.SSETransport {
		t.Fatalf("wrong failure: %v", failure)
	}
}

func TestSSEInvalidUTF8AndLargeEvent(t *testing.T) {
	for _, tc := range []struct {
		suffix string
		length string
	}{{"\xff\n\n", "1"}, {"\xc3", "1"}, {"\xe2\x82", "2"}, {"\xf0\x9f\x91", "3"}} {
		var failure error
		var received []string
		for data, err := range provider.SSEData(
			t.Context(),
			io.NopCloser(strings.NewReader("data: first\n\ndata: "+
				tc.suffix)),
		) {
			if err != nil {
				failure = err
			} else {
				received = append(received, data)
			}
		}
		requireEqual(t, received, []string{"first"})
		var stream *provider.SSEError
		if !errors.As(failure, &stream) || stream.Kind != provider.SSEUTF8 {
			t.Fatalf("expected UTF-8 failure, got %v", failure)
		}
		requireEqual(
			t,
			failure.Error(),
			"the event stream is not UTF-8: invalid utf-8 sequence of "+
				tc.length+
				" bytes from index 0",
		)
	}

	long := strings.Repeat("x", 128*1024)
	requireEqual(t, frames(t, []byte("data: "+long)), []string{long})
}

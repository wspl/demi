package commandservice_test

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"sync"
	"testing"
	"time"

	cs "github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

type logCapture struct {
	mu       sync.Mutex
	messages []string
}

func (h *logCapture) Enabled(context.Context, slog.Level) bool { return true }

func (h *logCapture) Handle(_ context.Context, r slog.Record) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.messages = append(h.messages, r.Message)
	return nil
}

func (h *logCapture) WithAttrs([]slog.Attr) slog.Handler { return h }

func (h *logCapture) WithGroup(string) slog.Handler { return h }

// Budget ten seconds. Both reservations must reach the callback before completion;
// the peer deliberately keeps the response open after its nonzero completion.
func TestNumbersDuplicateIDsCompletionAndDiagnostics(t *testing.T) {
	capture := &logCapture{}
	previous := slog.Default()
	slog.SetDefault(slog.New(capture))
	t.Cleanup(func() { slog.SetDefault(previous) })
	entered := make(chan struct{}, 2)
	client, ctx := scriptedPeer(t, func(w http.ResponseWriter, r *http.Request) {
		for range 2 {
			request, err := cs.Encode(cs.NumbersRequest{ID: 7, Conversation: "c1", Sequence: cs.Tab, Count: 1})
			if err != nil {
				t.Error(err)
				return
			}
			if !sendRecord(t, w, cs.Record{Kind: cs.Stdout, Data: request}) {
				return
			}
		}
		for range 2 {
			select {
			case <-entered:
			case <-r.Context().Done():
				return
			}
		}
		if !sendRecord(t, w, cs.Record{Kind: cs.Stderr, Data: []byte("diagnostic \n")}) {
			return
		}
		if !sendRecord(t, w, cs.Record{Kind: cs.Completed, Completion: cs.Completion{ExitCode: 1}}) {
			return
		}
		<-r.Context().Done()
	})
	numbers, err := client.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	err = numbers.Answer(ctx, func(ctx context.Context, _ cs.NumbersRequest) (uint64, error) {
		entered <- struct{}{}
		<-ctx.Done()
		return 0, ctx.Err()
	})
	if err != nil {
		t.Fatal(err)
	}
	capture.mu.Lock()
	defer capture.mu.Unlock()
	if len(capture.messages) != 1 || capture.messages[0] != "numbers stream: diagnostic" {
		t.Fatal(capture.messages)
	}
}

// Budget ten seconds; HTTP responses order malformed, successful, and duplicate opens.
func TestNumbersClaimRequiresValidMetadata(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	a, b := net.Pipe()
	done := make(chan error, 1)
	t.Cleanup(func() {
		cancel()
		// Teardown can race the server's own close.
		_ = a.Close()
		_ = b.Close()
		if err := <-done; err != nil {
			t.Error(err)
		}
	})
	go func() { done <- cs.Serve(ctx, a, servicetest.NewFixture()) }()
	protocols := new(http.Protocols)
	protocols.SetUnencryptedHTTP2(true)
	transport := &http.Transport{
		Protocols:   protocols,
		DialContext: func(context.Context, string, string) (net.Conn, error) { return b, nil },
	}
	client, err := transport.NewClientConn(ctx, "http", "demi:80")
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		metadata string
		status   int
	}{
		{`[]`, http.StatusBadRequest},
		{`{}`, http.StatusOK},
		{`[]`, http.StatusBadRequest},
		{`{}`, http.StatusConflict},
	} {
		data := make([]byte, 4)
		binary.BigEndian.PutUint32(data, uint32(len(test.metadata)))
		data = append(data, []byte(test.metadata)...)
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://demi"+cs.NumbersPath, bytes.NewReader(data))
		if err != nil {
			t.Fatal(err)
		}
		response, err := client.RoundTrip(request)
		if err != nil {
			t.Fatal(err)
		}
		if err = response.Body.Close(); err != nil {
			t.Fatal(err)
		}
		if response.StatusCode != test.status {
			t.Fatal(response.StatusCode, test.status)
		}
	}
}

func TestFixtureNumberLiteralSemantics(t *testing.T) {
	client, ctx := clientFor(t, servicetest.NewFixture())
	numbers, err := client.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	counts := make(chan int, 8)
	answered := make(chan error, 1)
	go func() {
		answered <- numbers.Answer(ctx, func(_ context.Context, request cs.NumbersRequest) (uint64, error) {
			counts <- request.Count
			return 1, nil
		})
	}()
	for _, test := range []struct {
		literal string
		count   int
	}{{"2", 2}, {"2.0", 1}, {"2e0", 1}, {"-1", 1}, {"18446744073709551616", 1}} {
		value := invocation("number")
		value.Args = []byte(`{"count":` + test.literal + `}`)
		_, _, completion := run(t, ctx, client, value, nil)
		if completion.ExitCode != 0 {
			t.Fatal(completion)
		}
		if count := <-counts; count != test.count {
			t.Fatal(test.literal, count)
		}
	}
	value := invocation("number")
	value.Args = []byte(`{"count":4294967296}`)
	_, _, completion := run(t, ctx, client, value, nil)
	if completion.Error == nil || completion.Error.Message != "out of range integral type conversion attempted" {
		t.Fatal(completion)
	}
	if err = client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err = <-answered; err != nil {
		t.Fatal(err)
	}
}

func TestNumbersErrorWords(t *testing.T) {
	client, ctx := clientFor(t, servicetest.NewFixture())
	numbers, err := client.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	go func() {
		answered <- numbers.Answer(
			ctx,
			func(context.Context, cs.NumbersRequest) (uint64, error) { return 0, errors.New("no numbers") },
		)
	}()
	_, _, completion := run(t, ctx, client, invocation("number"), nil)
	if completion.Error == nil || completion.Error.Message != "conversation numbers: no numbers" {
		t.Fatal(completion)
	}
	if err = client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err = <-answered; err != nil && !errors.Is(err, io.EOF) {
		t.Fatal(err)
	}
}

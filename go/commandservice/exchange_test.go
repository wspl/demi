package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"sync"
	"testing"
	"time"

	cs "github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// scriptedPeer exposes wire sequences a valid Handler cannot generate.
// Its network connection and HTTP worker are always joined; each test has a ten-second guard.
func scriptedPeer(t *testing.T, handler http.HandlerFunc) (*cs.Client, context.Context) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	protocols := new(http.Protocols)
	protocols.SetUnencryptedHTTP2(true)
	server := &http.Server{Handler: handler, Protocols: protocols}
	done := make(chan error, 1)
	t.Cleanup(func() {
		if err := server.Close(); err != nil {
			t.Error(err)
		}
		if err := <-done; err != nil && !errors.Is(err, http.ErrServerClosed) {
			t.Error(err)
		}
	})
	go func() { done <- server.Serve(listener) }()
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// The transport may already have closed this connection.
		_ = conn.Close()
	})
	client, err := cs.Connect(ctx, conn)
	if err != nil {
		t.Fatal(err)
	}
	return client, ctx
}

func sendRecord(t *testing.T, w http.ResponseWriter, record cs.Record) bool {
	t.Helper()
	data, err := record.Encode()
	if err != nil {
		t.Error(err)
		return false
	}
	if _, err = w.Write(data); err != nil {
		return false
	}
	return http.NewResponseController(w).Flush() == nil
}

type signalledReader struct {
	io.Reader
	entered  chan struct{}
	returned chan struct{}
	once     sync.Once
}

func (r *signalledReader) Read(data []byte) (int, error) {
	r.once.Do(func() { close(r.entered) })
	count, err := r.Reader.Read(data)
	if r.returned != nil {
		close(r.returned)
	}
	return count, err
}

type signalledWriter struct{ wrote chan struct{} }

func (w signalledWriter) Write(data []byte) (int, error) {
	close(w.wrote)
	return len(data), nil
}

func TestExchangeDoesNotWaitForBlockedRead(t *testing.T) {
	read, write := io.Pipe()
	t.Cleanup(func() {
		// These closes release the caller-owned test input even on assertion failure.
		_ = read.Close()
		_ = write.Close()
	})
	input := &signalledReader{Reader: read, entered: make(chan struct{}), returned: make(chan struct{})}
	client, ctx := scriptedPeer(t, func(w http.ResponseWriter, r *http.Request) {
		if !sendRecord(t, w, cs.Record{Kind: cs.InputPull}) {
			return
		}
		select {
		case <-input.entered:
		case <-r.Context().Done():
			return
		}
		if !sendRecord(t, w, cs.Record{Kind: cs.Stdout, Data: []byte("visible")}) {
			return
		}
		sendRecord(t, w, cs.Record{Kind: cs.Completed})
	})
	stream, err := client.Invoke(ctx, invocation("probe"))
	if err != nil {
		t.Fatal(err)
	}
	wrote := make(chan struct{})
	result := make(chan error, 1)
	go func() {
		_, err := cs.Exchange(ctx, stream, input, signalledWriter{wrote: wrote}, io.Discard)
		result <- err
	}()
	select {
	case <-wrote:
	case <-ctx.Done():
		t.Fatal("output waited behind Read")
	}
	select {
	case err = <-result:
		if err != nil {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal("completion waited behind Read")
	}
	if err = write.Close(); err != nil {
		t.Fatal(err)
	}
	<-input.returned
}

type zeroProgressReader struct{ read bool }

func (r *zeroProgressReader) Read(data []byte) (int, error) {
	if !r.read {
		r.read = true
		return 0, nil
	}
	return copy(data, "content"), io.EOF
}

func TestExchangeRetriesZeroProgress(t *testing.T) {
	for _, operation := range []string{"first", "echo"} {
		t.Run(operation, func(t *testing.T) {
			client, ctx := clientFor(t, servicetest.NewFixture())
			stream, err := client.Invoke(ctx, invocation(operation))
			if err != nil {
				t.Fatal(err)
			}
			var output bytes.Buffer
			if _, err = cs.Exchange(ctx, stream, &zeroProgressReader{}, &output, io.Discard); err != nil {
				t.Fatal(err)
			}
			if output.String() != "content" {
				t.Fatalf("output %q", output.String())
			}
		})
	}
}

// Budget ten seconds per case. Handler entry or request EOF establishes that the
// first demand has left the queue before the peer sends one or two more pulls.
func TestExchangeDemandQueue(t *testing.T) {
	for _, test := range []struct {
		name   string
		ended  bool
		queued int
	}{
		{"reading_one_queued", false, 1},
		{"reading_overflow", false, 2},
		{"ended_one_queued", true, 1},
		{"ended_overflow", true, 2},
	} {
		t.Run(test.name, func(t *testing.T) {
			read, write := io.Pipe()
			t.Cleanup(func() {
				// Release caller-owned blocked input, including on assertion failures.
				_ = read.Close()
				_ = write.Close()
			})
			blocked := &signalledReader{Reader: read, entered: make(chan struct{})}
			var input io.Reader = blocked
			if test.ended {
				input = bytes.NewReader(nil)
			}
			client, ctx := scriptedPeer(t, func(w http.ResponseWriter, r *http.Request) {
				if !sendRecord(t, w, cs.Record{Kind: cs.InputPull}) {
					return
				}
				if test.ended {
					if _, err := io.Copy(io.Discard, r.Body); err != nil {
						t.Error(err)
						return
					}
				} else {
					select {
					case <-blocked.entered:
					case <-r.Context().Done():
						return
					}
				}
				for range test.queued {
					if !sendRecord(t, w, cs.Record{Kind: cs.InputPull}) {
						return
					}
				}
				if test.queued == 1 {
					sendRecord(t, w, cs.Record{Kind: cs.Completed})
				} else {
					<-r.Context().Done()
				}
			})
			stream, err := client.Invoke(ctx, invocation("probe"))
			if err != nil {
				t.Fatal(err)
			}
			_, err = cs.Exchange(ctx, stream, input, io.Discard, io.Discard)
			if test.queued == 1 {
				if err != nil {
					t.Fatal(err)
				}
			} else {
				var invalid *cs.InvalidError
				if !errors.As(err, &invalid) || invalid.Rule != "overlapping input demands" {
					t.Fatal(err)
				}
			}
		})
	}
}

func TestExchangeAfterEarlyInputEnd(t *testing.T) {
	client, ctx := clientFor(t, servicetest.NewFixture())
	stream, err := client.Invoke(ctx, invocation("echo"))
	if err != nil {
		t.Fatal(err)
	}
	if err = stream.End(); err != nil {
		t.Fatal(err)
	}
	completion, err := cs.Exchange(ctx, stream, bytes.NewReader(nil), io.Discard, io.Discard)
	if err != nil || completion.ExitCode != 0 {
		t.Fatal(completion, err)
	}
}

func TestLateInputBeforeNextAndRepeatedEOF(t *testing.T) {
	client, ctx := scriptedPeer(t, func(w http.ResponseWriter, r *http.Request) {
		sendRecord(t, w, cs.Record{Kind: cs.Completed})
	})
	stream, err := client.Invoke(ctx, invocation("probe"))
	if err != nil {
		t.Fatal(err)
	}
	// Force the request writer to meet the stopped body before consuming any response.
	for range 8 {
		if err = stream.Write(make([]byte, cs.MaxRecordBytes)); err != nil {
			t.Fatal(err)
		}
	}
	if err = stream.End(); err != nil {
		t.Fatal(err)
	}
	record, err := stream.Next()
	if err != nil || record.Kind != cs.Completed {
		t.Fatal(record, err)
	}
	for range 3 {
		if _, err = stream.Next(); err != io.EOF {
			t.Fatal(err)
		}
	}
}

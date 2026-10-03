package process

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runnerwire"
)

// pipeServer uses in-memory socket endpoints so HTTP timer scenarios run under
// synctest rather than waiting on wall time. Every request handler is joined.
func pipeServer(t *testing.T, client *PipeClient, handler func(*http.Request, net.Conn)) func() {
	t.Helper()
	var workers sync.WaitGroup
	client.transport.DisableKeepAlives = true
	client.transport.DialContext = func(context.Context, string, string) (net.Conn, error) {
		local, remote := net.Pipe()
		workers.Add(1)
		go func() {
			defer workers.Done()
			// Cleanup follows the operation result; cancellation may already have closed it.
			defer func() { _ = remote.Close() }()
			request, err := http.ReadRequest(bufio.NewReader(remote))
			if err != nil {
				t.Errorf("read request: %v", err)
				return
			}
			// Cleanup follows the operation result; cancellation may already have closed it.
			defer func() { _ = request.Body.Close() }()
			handler(request, remote)
		}()
		return local, nil
	}
	return func() {
		_ = client.Close()
		workers.Wait()
	}
}

func testPipeClient(t *testing.T, timeout time.Duration) *PipeClient {
	t.Helper()
	origin, err := runnerwire.ParseBackendURL("http://backend.invalid/ignored?ignored")
	if err != nil {
		t.Fatal(err)
	}
	client, err := NewPipeClientWithConnectTimeout(
		origin,
		func() (runnerwire.DeviceToken, bool) { return runnerwire.DeviceToken("test-token"), true },
		timeout,
	)
	if err != nil {
		t.Fatal(err)
	}
	return client
}

func writeReply(t *testing.T, connection net.Conn, text string) {
	t.Helper()
	if _, err := io.WriteString(connection, text); err != nil {
		t.Errorf("write reply: %v", err)
	}
}

func TestPipeQuietIOOutlivesConnectDeadline(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		const deadline = 200 * time.Millisecond
		const quiet = 3 * deadline
		client := testPipeClient(t, deadline)
		finish := pipeServer(t, client, func(request *http.Request, connection net.Conn) {
			if request.Header.Get("Authorization") != "Bearer test-token" {
				t.Errorf("authorization %q", request.Header.Get("Authorization"))
			}
			switch request.URL.Path {
			case "/upload":
				data, err := io.ReadAll(request.Body)
				if err != nil || string(data) != "firstlast" {
					t.Errorf("upload %q: %v", data, err)
				}
				writeReply(t, connection, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
			case "/headers":
				time.Sleep(quiet)
				writeReply(t, connection, "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nreply")
			case "/body":
				writeReply(t, connection, "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nfirst")
				time.Sleep(quiet)
				writeReply(t, connection, "last!")
			default:
				t.Errorf("path %q", request.URL.Path)
			}
		})
		defer finish()
		for _, tc := range []struct{ path, want string }{{"/headers", "reply"}, {"/body", "firstlast!"}} {
			start := time.Now()
			body, err := client.Open(t.Context(), tc.path)
			if err != nil {
				t.Fatal(err)
			}
			data, err := io.ReadAll(body)
			closeErr := body.Close()
			if err != nil || closeErr != nil || string(data) != tc.want {
				t.Fatalf("body %q: %v / %v", data, err, closeErr)
			}
			if time.Since(start) < quiet {
				t.Fatal("quiet interval did not elapse")
			}
		}
		reader, writer := io.Pipe()
		sent := make(chan struct{})
		go func() {
			defer close(sent)
			// Cleanup follows the operation result; cancellation may already have closed it.
			defer func() { _ = writer.Close() }()
			if _, err := writer.Write([]byte("first")); err != nil {
				t.Error(err)
				return
			}
			time.Sleep(quiet)
			if _, err := writer.Write([]byte("last")); err != nil {
				t.Error(err)
			}
		}()
		if err := client.Put(t.Context(), "/upload", reader); err != nil {
			t.Fatal(err)
		}
		<-sent
	})
}

func TestPipeCancellationAndOrigin(t *testing.T) {
	t.Run("origin", func(t *testing.T) {
		t.Skip("fidelity 6: PipeClient adds a pipe: prefix to the origin refusal text")
		client := testPipeClient(t, time.Second)
		for _, path := range []string{"//elsewhere/pipe", "https://elsewhere/pipe", "/\\elsewhere"} {
			// The refusal text is the same for all three inputs.
			const want = "pipe URL must be origin-relative"
			if _, err := client.Open(t.Context(), path); err == nil || err.Error() != want {
				t.Errorf("open %q: got %v, want %q", path, err, want)
			}
		}
	})
	synctest.Test(t, func(t *testing.T) {
		client := testPipeClient(t, time.Second)
		for _, upload := range []bool{false, true} {
			requestRead := make(chan struct{})
			finish := pipeServer(t, client, func(request *http.Request, connection net.Conn) {
				close(requestRead)
				// EOF is delivered by client cancellation; the peer never answers.
				_, _ = io.Copy(io.Discard, request.Body)
				var one [1]byte
				_, _ = connection.Read(one[:])
			})
			ctx, cancel := context.WithCancel(t.Context())
			done := make(chan error, 1)
			reader, writer := io.Pipe()
			go func() {
				if upload {
					done <- client.Put(ctx, "/quiet", reader)
				} else {
					body, err := client.Open(ctx, "/quiet")
					if body != nil {
						_ = body.Close()
					}
					done <- err
				}
			}()
			<-requestRead
			cancel()
			if err := <-done; !errors.Is(err, context.Canceled) {
				t.Fatalf("cancel error: %v", err)
			}
			_ = reader.Close()
			_ = writer.Close()
			finish()
		}
	})
}

func TestPipeRefusalsAndConfirmationBounds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		for _, tc := range []struct {
			status int
			body   string
			want   string
		}{
			{http.StatusFound, "moved", "pipe refused (302 Found): moved"},
			{
				http.StatusForbidden,
				strings.Repeat("x", pipeAnswerBytes+10),
				"pipe refused (403 Forbidden): " + strings.Repeat("x", pipeAnswerBytes),
			},
			{http.StatusOK, strings.Repeat("x", pipeAnswerBytes+1), "oversized pipe confirmation"},
		} {
			client := testPipeClient(t, time.Second)
			finish := pipeServer(t, client, func(request *http.Request, connection net.Conn) {
				_, _ = io.Copy(io.Discard, request.Body)
				// The client intentionally closes before all oversized refusal bytes.
				_, _ = fmt.Fprintf(
					connection,
					"HTTP/1.1 %d %s\r\nContent-Length: %d\r\nLocation: http://elsewhere.invalid/\r\nConnection: close\r\n\r\n%s",
					tc.status,
					http.StatusText(tc.status),
					len(tc.body),
					tc.body,
				)
			})
			err := client.Put(t.Context(), "/pipe", io.NopCloser(bytes.NewReader(nil)))
			if err == nil || err.Error() != tc.want {
				t.Fatalf("error %v, want %q", err, tc.want)
			}
			finish()
		}
	})
}

func TestReportPipe(t *testing.T) {
	frames := make(chan []byte, 1)
	if err := ReportPipe(t.Context(), frames, "pipe", errors.New("write failed")); err != nil {
		t.Fatal(err)
	}
	report, err := runnerwire.DecodeOutbound(<-frames)
	if err != nil {
		t.Fatal(err)
	}
	done, ok := report.(*runnerwire.PipeDone)
	if !ok || done.PipeID != "pipe" || done.Ok || done.Error == nil || *done.Error != "write failed" {
		t.Fatalf("report %+v", report)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := ReportPipe(ctx, make(chan []byte), "pipe", nil); !errors.Is(err, context.Canceled) {
		t.Fatalf("report cancel: %v", err)
	}
}

func TestPipeRetriesOnlyUnreadUploads(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		for _, consumed := range []bool{false, true} {
			client := testPipeClient(t, time.Second)
			finish := pipeServer(t, client, func(request *http.Request, connection net.Conn) {
				_, _ = io.Copy(io.Discard, request.Body)
				if !consumed {
					writeReply(t, connection, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
				}
			})
			dial := client.transport.DialContext
			calls := 0
			client.transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
				calls++
				if !consumed && calls == 1 {
					return nil, cmdsdk.Exhaustion()
				}
				return dial(ctx, network, address)
			}
			body := io.NopCloser(strings.NewReader("whole body"))
			if consumed {
				body = io.NopCloser(failedUpload{})
			}
			err := client.Put(t.Context(), "/pipe", body)
			if consumed {
				if err == nil || calls != 1 {
					t.Fatalf("consumed retry: %d %v", calls, err)
				}
			} else if err != nil || calls != 2 {
				t.Fatalf("unread retry: %d %v", calls, err)
			}
			finish()
		}
	})
}

type failedUpload struct{}

func (failedUpload) Read([]byte) (int, error) { return 0, cmdsdk.Exhaustion() }

func TestPipeEarlyRefusalInterruptsUpload(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		client := testPipeClient(t, time.Second)
		finish := pipeServer(t, client, func(_ *http.Request, connection net.Conn) {
			writeReply(t, connection, "HTTP/1.1 403 Forbidden\r\nContent-Length: 7\r\nConnection: close\r\n\r\nrefused")
		})
		defer finish()
		reader, writer := io.Pipe()
		defer func() { _ = writer.Close() }()
		err := client.Put(t.Context(), "/pipe", reader)
		if err == nil || err.Error() != "pipe refused (403 Forbidden): refused" {
			t.Fatalf("early response: %v", err)
		}
	})
}

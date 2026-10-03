package edge

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http/httptest"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
func testPipe(t *testing.T) (*remotehost.PipeWriter, *remotehost.PipeReader) {
	t.Helper()
	broker := remotehost.NewPipes(120 * time.Second)
	t.Cleanup(func() {
		if err := broker.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	pipe := broker.Mint("", "")
	writer, err := pipe.Writer()
	if err != nil {
		t.Fatal(err)
	}
	reader, err := pipe.Reader()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		writer.Fail("test ended")
		_ = reader.Close(context.Background())
	})
	return writer, reader
}

func TestConnectionInactivityFollowsLastByte(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first, second := net.Pipe()
		// The operation reports IO failures; cleanup has no further recipient.
		defer func() { _ = second.Close() }()
		activity := newActivity(first)
		// The operation reports IO failures; cleanup has no further recipient.
		defer func() { _ = activity.Close() }()
		stop := activity.watch(t.Context(), transferIdle)
		defer stop()
		moved := make(chan struct{})
		go func() {
			timer := time.NewTimer(30 * time.Second)
			defer timer.Stop()
			<-timer.C
			activity.touch()
			close(moved)
		}()
		<-moved
		began := time.Now()
		_, err := second.Read(make([]byte, 1))
		if err == nil {
			t.Fatal("idle connection stayed open")
		}
		if time.Since(began) != transferIdle {
			t.Fatalf("closed after %v", time.Since(began))
		}
	})
}

type chunkBody struct{ chunks [][]byte }

func (b *chunkBody) Read(_ context.Context, p []byte) (int, error) {
	if len(b.chunks) == 0 {
		return 0, io.EOF
	}
	chunk := b.chunks[0]
	b.chunks = b.chunks[1:]
	return copy(p, chunk), nil
}
func (*chunkBody) Close(context.Context) error { return nil }
func TestUploadHostTimeDoesNotCountAgainstBrowser(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		writer, reader := testPipe(t)
		finished := make(chan struct{})
		var output bytes.Buffer
		go func() {
			defer close(finished)
			for {
				timer := time.NewTimer(35 * time.Second)
				<-timer.C
				timer.Stop()
				chunk, err := reader.Next(t.Context())
				if errors.Is(err, io.EOF) {
					return
				}
				if err != nil {
					t.Error(err)
					return
				}
				output.Write(chunk)
			}
		}()
		body := &chunkBody{chunks: [][]byte{[]byte("one "), []byte("two "), []byte("three")}}
		start := time.Now()
		err := copyUpload(t.Context(), body, writer, func(ctx context.Context) error {
			select {
			case <-finished:
				return nil
			case <-ctx.Done():
				return ctx.Err()
			}
		})
		if err != nil {
			t.Fatal(err)
		}
		<-finished
		if output.String() != "one two three" || time.Since(start) <= transferIdle {
			t.Fatalf("%q in %v", output.String(), time.Since(start))
		}
	})
}

type quietBody struct{}

func (quietBody) Read(ctx context.Context, _ []byte) (int, error) {
	<-ctx.Done()
	return 0, ctx.Err()
}
func (quietBody) Close(context.Context) error { return nil }
func TestQuietBrowserStallsUploadAndRevocationRefusesIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		writer, reader := testPipe(t)
		started := time.Now()
		err := copyUpload(t.Context(), quietBody{}, writer, func(context.Context) error {
			t.Error("stalled upload committed")
			return nil
		})
		var failure *apiError
		if !errors.As(err, &failure) || failure.body.Code != "transfer_stalled" || time.Since(started) != transferIdle {
			t.Fatal(err, time.Since(started))
		}
		if _, err := reader.Next(t.Context()); err == nil {
			t.Fatal("stalled upload did not fail Host pipe")
		}
		writer, _ = testPipe(t)
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan error, 1)
		go func() { done <- copyUpload(ctx, quietBody{}, writer, func(context.Context) error { return nil }) }()
		synctest.Wait()
		cancel()
		err = <-done
		if !errors.As(err, &failure) || failure.body.Code != "conversation_busy" {
			t.Fatal(err)
		}
	})
}

// A lease's only edge-visible effects are cancellation and release. The
// hostaccess implementation owns how releasing it admits the next operation.
type testDownloadLease struct {
	ctx      context.Context
	released chan struct{}
}

func (l *testDownloadLease) Context() context.Context { return l.ctx }
func (l *testDownloadLease) Release()                 { close(l.released) }

func TestDownloadEndsCompleteOnlyWhenHostReadDid(t *testing.T) {
	for _, scenario := range []string{"complete", "host_failed", "visitor_left", "revoked"} {
		t.Run(scenario, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				writer, reader := testPipe(t)
				first, second := net.Pipe()
				peer := newActivity(first)
				defer func() {
					_ = peer.Close()
					_ = second.Close()
				}()
				request, cancelRequest := context.WithCancel(t.Context())
				defer cancelRequest()
				lifetime, cancelLease := context.WithCancel(t.Context())
				defer cancelLease()
				lease := &testDownloadLease{ctx: lifetime, released: make(chan struct{})}
				response := httptest.NewRecorder()
				done := make(chan error, 1)
				go func() { done <- serveDownload(request, response, peer, reader, lease, 200) }()
				switch scenario {
				case "complete":
					for _, chunk := range []string{"one ", "two"} {
						if err := writer.Write(t.Context(), []byte(chunk)); err != nil {
							t.Fatal(err)
						}
					}
					writer.End()
				case "host_failed":
					if err := writer.Write(t.Context(), []byte("part")); err != nil {
						t.Fatal(err)
					}
					writer.Fail("the device went away")
				case "visitor_left":
					cancelRequest()
				case "revoked":
					cancelLease()
				}
				err := <-done
				<-lease.released
				if scenario == "visitor_left" {
					if err := writer.Write(t.Context(), []byte("late")); err == nil {
						t.Fatal("departed visitor still accepts Host bytes")
					}
				}
				if scenario == "complete" {
					if err != nil || response.Body.String() != "one two" {
						t.Fatal(err, response.Body.String())
					}
				} else {
					if err == nil {
						t.Fatal("interrupted transfer completed")
					}
					closedAt := time.Now()
					if _, err := second.Read(make([]byte, 1)); !errors.Is(err, io.EOF) || time.Now() != closedAt {
						t.Fatalf("interrupted connection was not closed immediately: %v", err)
					}
				}
			})
		})
	}
}

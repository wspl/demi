package usershard

import (
	"bufio"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/webapi"
)

// Virtual time and an in-memory page transport check both a blocked frame and
// a peer that reads the close without acknowledging it. No wall time passes.
func TestPageCloseInterruptsTransportAtBound(t *testing.T) {
	for _, name := range []string{"unacknowledged close", "stalled write", "queued frame"} {
		t.Run(name, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				transport, peer := net.Pipe()
				// The page may already have ended the in-memory connection.
				defer func() { _ = peer.Close() }()
				recorder := &pageSocketRecorder{httptest.NewRecorder(), transport}
				request := httptest.NewRequest(http.MethodGet, "http://page.test", nil)
				request.Header.Set("Connection", "Upgrade")
				request.Header.Set("Upgrade", "websocket")
				request.Header.Set("Sec-WebSocket-Version", "13")
				request.Header.Set("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
				socket, err := websocket.Accept(recorder, request, nil)
				if err != nil {
					_ = transport.Close()
					t.Fatal(err)
				}
				// The bounded close owns transport termination; cleanup is idempotent.
				defer func() { _ = socket.CloseNow() }()
				if name == "queued frame" {
					written := make(chan error, 1)
					go func() {
						// Two empty masked binary messages; reading one leaves the next buffered.
						_, err := peer.Write([]byte{0x82, 0x80, 0, 0, 0, 0, 0x82, 0x80, 0, 0, 0, 0})
						written <- err
					}()
					_, _, err := socket.Read(t.Context())
					if err != nil {
						t.Fatal(err)
					}
					if err := <-written; err != nil {
						t.Fatal(err)
					}
				}
				page := newPageSocket(NewPageConnection(socket, transport), DefaultPageTuning())
				page.tuning.CloseWait = 100 * time.Millisecond
				done := make(chan struct{})
				go func() {
					defer close(done)
					// Closing the transport ends either operation; its error is expected.
					if name != "unacknowledged close" {
						_ = page.send(t.Context(), &framewire.HeartbeatFrame{})
						return
					}
					_, _ = io.Copy(io.Discard, peer)
				}()
				synctest.Wait()
				started := time.Now()
				page.close(t.Context(), websocket.StatusGoingAway, "backend_closing")
				<-done
				if elapsed := time.Since(started); elapsed != page.tuning.CloseWait {
					t.Fatalf("page close duration: got %s, want %s", elapsed, page.tuning.CloseWait)
				}
			})
		})
	}
}

// pageSocketRecorder supplies the hijacked page transport without real sockets.
type pageSocketRecorder struct {
	*httptest.ResponseRecorder
	transport net.Conn
}

func (r *pageSocketRecorder) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	return r.transport, bufio.NewReadWriter(bufio.NewReader(r.transport), bufio.NewWriter(r.transport)), nil
}

// One loopback socket per case exposes cancellation with an already-ready
// outbox event, without a model, storage, or a timer wait.
func TestConversationShutdownSendsCloseWithReadyOutbox(t *testing.T) {
	for _, buffered := range []bool{false, true} {
		name := "ended"
		if buffered {
			name = "buffered"
		}
		t.Run(name, func(t *testing.T) {
			done := make(chan struct{})
			transports := make(chan net.Conn, 1)
			listener := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				defer close(done)
				socket, err := websocket.Accept(w, r, nil)
				if err != nil {
					t.Error(err)
					return
				}
				// The exchange may already have closed the connection.
				defer func() { _ = socket.CloseNow() }()
				page := newPageSocket(NewPageConnection(socket, <-transports), DefaultPageTuning())
				defer page.heartbeat.Stop()
				outgoing := make(chan framewire.ServerFrame, 1)
				if buffered {
					outgoing <- &framewire.HeartbeatFrame{}
				}
				close(outgoing)
				ctx, cancel := context.WithCancel(r.Context())
				cancel()
				// Shutdown and the outbox are both ready before the relay selects.
				// Its return may report cancellation; the page must see the close.
				_ = (&Shard{}).exchangeConversation(ctx, conversationExchange{
					page: page, outgoing: outgoing, frames: &server.Frames{},
				})
			}))
			listener.Config.ConnContext = func(ctx context.Context, transport net.Conn) context.Context {
				transports <- transport
				return ctx
			}
			listener.Start()
			defer listener.Close()
			socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(listener.URL, "http"), nil)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				_ = socket.CloseNow()
				<-done
			}()
			for err == nil {
				_, _, err = socket.Read(t.Context())
			}
			var closed websocket.CloseError
			if !errors.As(err, &closed) || closed.Code != websocket.StatusGoingAway ||
				closed.Reason != "backend_closing" {
				t.Fatalf("shutdown close: got %v, want 1001 backend_closing", err)
			}
		})
	}
}

// This uses one loopback socket, no model or timer wait.
func TestPageSocketRejectsOversizedMessage(t *testing.T) {
	result := make(chan error, 1)
	done := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(done)
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			result <- err
			return
		}
		// The peer may have already closed the connection under test.
		defer func() {
			_ = socket.CloseNow()
		}()
		_, _, err = ReadPageMessage(r.Context(), socket)
		result <- err
	}))
	defer server.Close()
	socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	// The peer may have already closed the connection under test.
	defer func() {
		_ = socket.CloseNow()
		<-done
	}()
	// The peer may close while Write is completing; the receiving refusal is the assertion.
	_ = socket.Write(t.Context(), websocket.MessageText, make([]byte, webapi.MaxPageMessageBytes+1))
	_, _, err = socket.Read(t.Context())
	if err == nil || websocket.CloseStatus(err) != -1 {
		t.Fatalf("oversized page message: %v", err)
	}
	if err := <-result; !errors.Is(err, websocket.ErrMessageTooBig) {
		t.Fatalf("oversized message error: %v", err)
	}
}

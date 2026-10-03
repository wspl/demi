package edge

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Each kind uses its production page upgrade and receiver on a real listener.
// Exact-limit binary messages reach the receiver's normal behavior; one byte
// over must end the transport without a close code. No models or sleeps;
// expected cost is under one second without race instrumentation.
func TestPageMessageLimitForEverySocketKind(t *testing.T) {
	for _, kind := range []string{"conversation", "sync", "user stream"} {
		t.Run(kind, func(t *testing.T) {
			for _, size := range []int{webapi.MaxPageMessageBytes, webapi.MaxPageMessageBytes + 1} {
				name := "exact limit"
				if size > webapi.MaxPageMessageBytes {
					name = "over limit"
				}
				t.Run(name, func(t *testing.T) {
					services := usershardtest.StartServices(t)
					user := databasetest.Master(t.Context(), t, services.Control)
					shards := usershardtest.StartShards(t, services)
					shard, err := shards.Of(t.Context(), user.ID)
					if err != nil {
						t.Fatal(err)
					}
					id := webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01")
					if _, _, err := services.Control.CreateConversation(t.Context(), user.ID, id); err != nil {
						t.Fatal(err)
					}
					record, err := services.Control.Conversation(t.Context(), id)
					if err != nil {
						t.Fatal(err)
					}
					expires, err := core.TimestampFromTime(time.Now().Add(time.Hour))
					if err != nil {
						t.Fatal(err)
					}
					toHost, hostInput := testPipe(t)
					hostOutput, fromHost := testPipe(t)
					lease, _ := hostaccess.NewLease(t.Context())
					defer lease.Release()
					stream := &hostaccess.UserStream{ToHost: toHost, FromHost: fromHost, Lease: lease}
					consumed := make(chan int, 1)
					readCtx, cancel := context.WithCancel(t.Context())
					go func() {
						total := 0
						defer func() { consumed <- total }()
						for total < webapi.MaxPageMessageBytes {
							data, err := hostInput.Next(readCtx)
							if err != nil {
								return
							}
							total += len(data)
						}
						hostOutput.End()
					}()
					defer func() {
						cancel()
						<-consumed
					}()
					done := make(chan struct{})
					server := httptest.NewUnstartedServer(
						http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
							defer close(done)
							defer toHost.Fail("page ended")
							socket, err := pageUpgrade(w, r, "page socket")
							if err != nil {
								t.Error(err)
								return
							}
							defer func() { _ = socket.CloseNow() }()
							// Serving may return the expected overflow error; the peer close
							// below is the assertion at the socket boundary.
							switch kind {
							case "conversation":
								_ = shard.ServeConversationSocket(r.Context(), *record, socket)
							case "sync":
								_ = shard.ServeSyncChannel(
									r.Context(),
									socket,
									usershard.ChannelSession{User: user, ExpiresAt: expires},
								)
							case "user stream":
								relayUserStream(
									r.Context(),
									socket,
									stream,
									make(chan struct{}),
									services.Pages.CloseWait,
								)
							}
						}),
					)
					services.PublicURL.Listening(nil, server.Listener.Addr().(*net.TCPAddr).AddrPort())
					server.Start()
					defer server.Close()
					socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil)
					if err != nil {
						t.Fatal(err)
					}
					defer func() {
						_ = socket.CloseNow()
						<-done
					}()
					// The peer may end the socket while Write is still finishing. Its received
					// close status, not a successful final write, is the observable contract.
					_ = socket.Write(t.Context(), websocket.MessageBinary, make([]byte, size))
					for {
						_, _, err = socket.Read(t.Context())
						if err != nil {
							break
						}
					}
					want := websocket.StatusCode(-1)
					if size == webapi.MaxPageMessageBytes {
						switch kind {
						case "conversation":
							want = websocket.StatusInvalidFramePayloadData
						case "sync":
							want = websocket.StatusPolicyViolation
						case "user stream":
							want = websocket.StatusNormalClosure
						}
					}
					if got := websocket.CloseStatus(err); got != want {
						t.Fatalf("close status=%d, want %d: %v", got, want, err)
					}
				})
			}
		})
	}
}

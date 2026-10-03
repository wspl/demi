package edge

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/hostaccess"
)

// Cost: one loopback WebSocket per scenario, no processes or timed waits.
func TestUserStreamCloseDistinguishesReleaseFromRevocation(t *testing.T) {
	for _, revoked := range []bool{false, true} {
		name := "completed after release"
		if revoked {
			name = "revoked before completed"
		}
		t.Run(name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			lease := hostaccess.NewLease(ctx)
			defer lease.Release()
			input, _ := testPipe(t)
			output, reader := testPipe(t)
			stream := &hostaccess.UserStream{ToHost: input, FromHost: reader, Lease: lease}
			defer input.Fail("test ended")
			defer func() { _ = reader.Close(context.Background()) }()
			output.End()
			// Force release before the edge can read EOF; no scheduler race is needed.
			lease.Release()
			if revoked {
				cancel()
			}
			done := make(chan struct{})
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				defer close(done)
				socket, err := pageUpgrade(w, r, "user stream")
				if err != nil {
					t.Error(err)
					return
				}
				defer func() { _ = socket.CloseNow() }()
				relayUserStream(r.Context(), socket, stream, nil, time.Second)
			}))
			defer server.Close()
			socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				_ = socket.CloseNow()
				<-done
			}()
			_, _, err = socket.Read(t.Context())
			want := websocket.CloseError{Code: websocket.StatusNormalClosure, Reason: "completed"}
			if revoked {
				want = websocket.CloseError{Code: 4000, Reason: "conversation_changed"}
			}
			var got websocket.CloseError
			if !errors.As(err, &got) || got != want {
				t.Fatalf("close = %v, want %v", err, want)
			}
		})
	}
}

package edge_test

import (
	"context"
	"io"
	"net/http"
	"net/http/httptrace"
	"net/netip"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/edge"

	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
)

// Real HTTP connections, no sleeps or external services. Two requests per case
// protect the client's reuse decision; expected cost is under one second.
func TestRefusedBodyAnnouncesWhetherConnectionCanBeReused(t *testing.T) {
	state := edge.AppState{Services: &usershard.Services{PublicURL: &runners.PublicURL{}}, Site: &edge.Site{}}
	e, err := edge.Start(t.Context(), netip.MustParseAddrPort("127.0.0.1:0"), state, "")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := e.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	for _, test := range []struct {
		name   string
		path   string
		status int
		close  bool
	}{
		{"unread body", "/not-a-route", http.StatusNotFound, true},
		{"consumed invalid body", "/api/setup", http.StatusBadRequest, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			transport := &http.Transport{MaxConnsPerHost: 1}
			defer transport.CloseIdleConnections()
			client := &http.Client{Transport: transport}
			endpoint := "http://" + e.LocalAddr().String()
			first, err := client.Post(endpoint+test.path, "application/json", strings.NewReader("{"))
			if err != nil {
				t.Fatal(err)
			}
			_, readErr := io.Copy(io.Discard, first.Body)
			closeErr := first.Body.Close()
			if readErr != nil || closeErr != nil {
				t.Fatal(readErr, closeErr)
			}
			if first.StatusCode != test.status || first.Close != test.close {
				t.Errorf(
					"first response: status=%d close=%v; want status=%d close=%v",
					first.StatusCode,
					first.Close,
					test.status,
					test.close,
				)
			}
			var reused atomic.Bool
			trace := &httptrace.ClientTrace{GotConn: func(info httptrace.GotConnInfo) { reused.Store(info.Reused) }}
			ctx := httptrace.WithClientTrace(t.Context(), trace)
			request, err := http.NewRequestWithContext(
				ctx,
				http.MethodPost,
				endpoint+"/not-a-route",
				strings.NewReader("{}"),
			)
			if err != nil {
				t.Fatal(err)
			}
			second, err := client.Do(request)
			if err != nil {
				t.Fatalf("second request after refusal: %v", err)
			}
			_, readErr = io.Copy(io.Discard, second.Body)
			closeErr = second.Body.Close()
			if readErr != nil || closeErr != nil || second.StatusCode != http.StatusNotFound {
				t.Fatal(second.StatusCode, readErr, closeErr)
			}
			if got := reused.Load(); got == test.close {
				t.Errorf("second connection reused=%v; first response close=%v", got, test.close)
			}
		})
	}
}

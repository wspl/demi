package backend_test

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/gates"
	exposeplugin "github.com/wspl/demi/internal/plugins/expose"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// TestExposeUnavailableWithoutDomain uses one paired runner; no service or model is needed to observe
// configuration refusal.
func TestExposeUnavailableWithoutDomain(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	_, err = backendtest.CreateExpose(ctx, b.Backend, s.User.ID, laptop.ID(), "1234")
	if !errors.Is(err, expose.ErrUnavailable) {
		t.Fatalf("create expose: %v", err)
	}
	page, err := b.Sync(ctx, t, &s)
	wireMust(t, err)
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	exposes, err := exposeplugin.DecodeExposeState(state.PluginStates["expose"])
	wireMust(t, err)
	if exposes.Available || len(exposes.Exposes) != 0 {
		t.Fatalf("expose state: %+v", exposes)
	}
	wireMust(t, page.Close(ctx))
	wireMust(t, b.Close(ctx))
}

const filesExposeDomain = "expose.localhost"

// filesExposeConfig enables public URLs for the fixture's loopback relay.
func filesExposeConfig(t *testing.T, h *backendtest.Harness) {
	t.Helper()
	domain, err := expose.ParseDomain(filesExposeDomain)
	wireMust(t, err)
	h.Config.ExposeDomain = &domain
}

// filesExposeService serves finite and held responses on the runner's loopback network.
func filesExposeService(t *testing.T, proceed ...<-chan struct{}) (*httptest.Server, <-chan struct{}) {
	t.Helper()
	released := make(chan struct{}, 128)
	service := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/hold" {
			w.Header().Set("Content-Type", "text/event-stream")
			_, err := io.WriteString(w, "held\n")
			if err != nil {
				return
			}
			w.(http.Flusher).Flush()
			var more <-chan struct{}
			if len(proceed) > 0 {
				more = proceed[0]
			}
			for {
				select {
				case <-r.Context().Done():
					released <- struct{}{}
					return
				case <-more:
					if _, err := io.WriteString(w, "still\n"); err != nil {
						return
					}
					w.(http.Flusher).Flush()
				}
			}
		}
		_, _ = io.WriteString(w, "hello") // A disconnected visitor may end the fixture response.
	}))
	t.Cleanup(service.Close)
	return service, released
}

// filesExposePort names the service endpoint in the same device-local form as the agent.
func filesExposePort(t *testing.T, s *httptest.Server) string {
	t.Helper()
	u, err := url.Parse(s.URL)
	wireMust(t, err)
	return u.Port()
}

// filesExposeState reads the plugin-owned list from the shared product channel.
func filesExposeState(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
) exposeplugin.ExposeState {
	t.Helper()
	page, snapshot := conversationPage(ctx, t, b, s)
	wireMust(t, page.Close(ctx))
	state, err := exposeplugin.DecodeExposeState(snapshot.PluginStates["expose"])
	wireMust(t, err)
	return state
}

// filesExpose creates a record, then observes the entry as its owner sees it.
func filesExpose(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	device webapi.DeviceID,
	port string,
) exposeplugin.ExposeEntry {
	t.Helper()
	created, err := backendtest.CreateExpose(ctx, b.Backend, s.User.ID, device, port)
	wireMust(t, err)
	for _, entry := range filesExposeState(ctx, t, b, s).Exposes {
		if entry.ID == created.Record.ID {
			return entry
		}
	}
	t.Fatal("new expose missing from plugin state")
	return exposeplugin.ExposeEntry{}
}

// filesExposeFetch connects locally while preserving the public URL's Host header.
func filesExposeFetch(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	entry exposeplugin.ExposeEntry,
	path string,
) *http.Response {
	t.Helper()
	public, err := url.Parse(entry.URL)
	wireMust(t, err)
	request, err := http.NewRequestWithContext(ctx, "GET", b.URL+path, nil)
	wireMust(t, err)
	request.Host = public.Host
	response, err := b.HTTP.Do(request)
	wireMust(t, err)
	return response
}

// filesExposeHeld waits for the first streamed bytes and owns the visitor connection.
func filesExposeHeld(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	entry exposeplugin.ExposeEntry,
) io.ReadCloser {
	t.Helper()
	response := filesExposeFetch(ctx, t, b, entry, "/hold")
	conversationEqual(t, response.StatusCode, 200)
	t.Cleanup(func() { _ = response.Body.Close() }) // Idempotent if the visitor already disconnected.
	first := make([]byte, 5)
	_, err := io.ReadFull(response.Body, first)
	wireMust(t, err)
	conversationEqual(t, string(first), "held\n")
	return response.Body
}

// filesExposeEnded observes both relay closure and the service's released connection.
func filesExposeEnded(ctx context.Context, t *testing.T, body io.ReadCloser, released <-chan struct{}) {
	t.Helper()
	_, err := io.ReadAll(body)
	if err != nil && !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatal(err)
	}
	wireMust(t, body.Close())
	select {
	case <-released:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
}

// filesExposeCall invokes an owner action and checks its plugin-specific refusal.
func filesExposeCall(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	method string,
	id webapi.ExposeID,
	status int,
) {
	t.Helper()
	answer := conversationRequest(
		ctx,
		t,
		b,
		s,
		"POST",
		"/api/plugins/expose/calls/"+method,
		conversationJSON(t, exposeplugin.ExposeCall{Expose: string(id)}),
		status,
	)
	if status != 200 {
		refusal, err := answer.ErrorBody()
		wireMust(t, err)
		conversationEqual(t, refusal.Code, webapi.ErrorCodePluginRefused)
		if refusal.Reason == nil || *refusal.Reason != "expose_not_found" {
			t.Fatalf("refusal: %+v", refusal)
		}
	}
}

// TestExposeOwnerControlsLifetimeAndVisitorsNeedNoSession uses a real relay to verify owner isolation, renewals,
// expiry and monotonically assigned numbers.
func TestExposeOwnerControlsLifetimeAndVisitorsNeedNoSession(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	service, _ := filesExposeService(t)
	port := filesExposePort(t, service)
	entry := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	address, err := url.Parse(b.URL)
	wireMust(t, err)
	conversationEqual(t, entry.URL, "http://"+string(entry.ID)+"."+filesExposeDomain+":"+address.Port()+"/")
	conversationEqual(t, string(entry.Address), "127.0.0.1:"+port)
	now, err := h.Clock.Now().Time()
	wireMust(t, err)
	expiry, err := entry.ExpiresAt.Time()
	wireMust(t, err)
	conversationEqual(t, expiry, now.Add(time.Hour))
	conversationEqual(t, entry.Number, uint64(1))
	state := filesExposeState(ctx, t, b, &s)
	conversationEqual(t, state.Available, true)
	conversationEqual(t, state.Exposes, []exposeplugin.ExposeEntry{entry})
	answer, err := backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	conversationEqual(t, string(answer.Body), "hello")
	wireMust(t, h.AddUser(ctx, "other@example.test", "other-pass-1", webapi.RoleUser))
	other, err := b.Login(ctx, "other@example.test", "other-pass-1")
	wireMust(t, err)
	conversationEqual(t, len(filesExposeState(ctx, t, b, &other).Exposes), 0)
	for _, method := range []string{"renew", "remove"} {
		filesExposeCall(ctx, t, b, &other, method, entry.ID, 409)
	}
	_, err = backendtest.CreateExpose(ctx, b.Backend, other.User.ID, laptop.ID(), "8080")
	if !errors.Is(err, expose.ErrDeviceNotFound) {
		t.Fatalf("foreign device: %v", err)
	}
	wireMust(t, h.Clock.Advance(50*time.Minute))
	filesExposeCall(ctx, t, b, &s, "renew", entry.ID, 200)
	renewed := filesExposeState(ctx, t, b, &s).Exposes[0]
	now, err = h.Clock.Now().Time()
	wireMust(t, err)
	expiry, err = renewed.ExpiresAt.Time()
	wireMust(t, err)
	conversationEqual(t, expiry, now.Add(time.Hour))
	wireMust(t, h.Clock.Advance(50*time.Minute))
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	wireMust(t, h.Clock.Advance(10*time.Minute+time.Second))
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	filesContains(t, string(answer.Body), "does not exist")
	db, err := h.ControlDatabase(ctx, t)
	wireMust(t, err)
	var records int
	wireMust(t, db.QueryRowContext(ctx, "SELECT COUNT(*) FROM exposes").Scan(&records))
	conversationEqual(t, records, 0)
	filesExposeCall(ctx, t, b, &s, "renew", entry.ID, 409)
	removed := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	conversationEqual(t, removed.Number, uint64(2))
	filesExposeCall(ctx, t, b, &s, "remove", removed.ID, 200)
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, removed, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	conversationEqual(t, len(filesExposeState(ctx, t, b, &s).Exposes), 0)
	filesExposeCall(ctx, t, b, &s, "remove", removed.ID, 409)
	wireMust(t, b.Close(ctx))
}

// TestExposeConnectionsEndWithRecordAndOfflineDeviceKeepsRecords holds real connections until removal, expiry
// and revocation; offline records survive.
func TestExposeConnectionsEndWithRecordAndOfflineDeviceKeepsRecords(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	service, released := filesExposeService(t)
	port := filesExposePort(t, service)
	removed := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	held := filesExposeHeld(ctx, t, b, removed)
	filesExposeCall(ctx, t, b, &s, "remove", removed.ID, 200)
	filesExposeEnded(ctx, t, held, released)
	expiring := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	wireMust(t, h.Clock.Advance(time.Hour-time.Second))
	held = filesExposeHeld(ctx, t, b, expiring)
	wireMust(t, h.Clock.Advance(2*time.Second))
	filesExposeEnded(ctx, t, held, released)
	db, err := h.ControlDatabase(ctx, t)
	wireMust(t, err)
	var records int
	wireMust(t, db.QueryRowContext(ctx, "SELECT COUNT(*) FROM exposes").Scan(&records))
	conversationEqual(t, records, 0)
	kept := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	wireMust(t, laptop.Runner.Kill(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, laptop.ID(), false))
	answer, err := backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, kept, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 502)
	filesContains(t, string(answer.Body), "device_offline")
	conversationEqual(t, filesExposeState(ctx, t, b, &s).Exposes, []exposeplugin.ExposeEntry{kept})
	_, err = backendtest.CreateExpose(ctx, b.Backend, s.User.ID, laptop.ID(), port)
	if !errors.Is(err, expose.ErrDeviceOffline) {
		t.Fatalf("offline expose: %v", err)
	}
	wireMust(t, laptop.Runner.StartAgain(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, laptop.ID(), true))
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, kept, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	conversationEqual(t, string(answer.Body), "hello")
	held = filesExposeHeld(ctx, t, b, kept)
	conversationRequest(ctx, t, b, &s, "DELETE", "/api/devices/"+string(laptop.ID()), "", 204)
	filesExposeEnded(ctx, t, held, released)
	conversationEqual(t, len(filesExposeState(ctx, t, b, &s).Exposes), 0)
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, kept, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	wireMust(t, b.Close(ctx))
}

// TestExposeShedsSixtyFifthConnectionAndReusesClosedPlace fills admission with sixty-four held real relays; the
// service's close event frees a place.
func TestExposeShedsSixtyFifthConnectionAndReusesClosedPlace(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	service, released := filesExposeService(t)
	entry := filesExpose(ctx, t, b, &s, laptop.ID(), filesExposePort(t, service))
	held := make([]io.ReadCloser, 0, 64)
	for range 64 {
		held = append(held, filesExposeHeld(ctx, t, b, entry))
	}
	answer, err := backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 503)
	filesContains(t, string(answer.Body), "connection limit")
	wireMust(t, held[len(held)-1].Close())
	held = held[:len(held)-1]
	select {
	case <-released:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	answer, err = backendtest.ReadAnswer(ctx, filesExposeFetch(ctx, t, b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	for _, body := range held {
		wireMust(t, body.Close())
	}
	wireMust(t, b.Close(ctx))
}

// TestExposeQuietConnectionEndsAtIdleLimit observes the real relay's one-second idle timeout through connection
// closure.
func TestExposeQuietConnectionEndsAtIdleLimit(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	h.Config.Exposes.Idle = time.Second
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	service, released := filesExposeService(t)
	entry := filesExpose(ctx, t, b, &s, laptop.ID(), filesExposePort(t, service))
	held := filesExposeHeld(ctx, t, b, entry)
	quiet := time.Now()
	filesExposeEnded(ctx, t, held, released)
	if elapsed := time.Since(quiet); elapsed < 900*time.Millisecond {
		t.Fatalf("closed after %s", elapsed)
	}
	wireMust(t, b.Close(ctx))
}

// TestExposeAgentAddsListsRenewsAndRemoves uses three real shell turns to exercise the expose CLI against the
// plugin's page state.
func TestExposeAgentAddsListsRenewsAndRemoves(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file", func(h *backendtest.Harness) { filesExposeConfig(t, h) })
	service, _ := filesExposeService(t)
	port := filesExposePort(t, service)
	add := "demi expose add " + port
	w.vendor.Respond(conversationShell(t, "t1", add+" && "+add, 20000))
	w.vendor.Respond(conversationAnswer(t, []string{"exposed"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	entries := filesExposeState(w.ctx, t, w.backend, &w.session).Exposes
	conversationEqual(t, len(entries), 2)
	var first, second exposeplugin.ExposeEntry
	for _, entry := range entries {
		if entry.Number == 1 {
			first = entry
		}
		if entry.Number == 2 {
			second = entry
		}
	}
	conversationEqual(t, []uint64{first.Number, second.Number}, []uint64{1, 2})
	result := conversationToolResult(t, w.vendor.Requests()[1], "t1")
	for _, entry := range entries {
		filesContains(
			t,
			toolstest.ShownOutput(result),
			fmt.Sprintf(
				"Exposed 127.0.0.1:%s on laptop as %s\nExpires in 60 minutes (expose %d).\n",
				port,
				entry.URL,
				entry.Number,
			),
		)
	}
	answer, err := backendtest.ReadAnswer(w.ctx, filesExposeFetch(w.ctx, t, w.backend, first, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	conversationEqual(t, string(answer.Body), "hello")
	w.vendor.Respond(conversationShell(t, "t2", "demi expose list && demi expose list --json", 20000))
	w.vendor.Respond(conversationAnswer(t, []string{"listed"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "go")
	wireMust(t, err)
	requests := w.vendor.Requests()
	result = conversationToolResult(t, requests[len(requests)-1], "t2")
	filesContains(t, result, fmt.Sprintf("Expose  Device  %-*s  Expires  URL\n", len(first.Address), "Address"))
	var lines []exposeplugin.ExposeLine
	for _, entry := range entries {
		filesContains(t, result, fmt.Sprintf("%-6d  laptop  %s  60 min   %s\n", entry.Number, entry.Address, entry.URL))
		lines = append(
			lines,
			exposeplugin.ExposeLine{
				Number:    entry.Number,
				Device:    "laptop",
				Address:   string(entry.Address),
				URL:       entry.URL,
				ExpiresAt: entry.ExpiresAt,
			},
		)
	}
	// The command's generated JSON shape is compared as an ordered value, not reconstructed.
	filesContains(t, result, conversationJSON(t, exposeplugin.ExposeLines{Exposes: lines}))
	wireMust(t, w.harness.AddUser(w.ctx, "other@example.test", "other-pass-1", webapi.RoleUser))
	other, err := w.backend.Login(w.ctx, "other@example.test", "other-pass-1")
	wireMust(t, err)
	desk, err := w.backend.Pair(w.ctx, t, &other, "desk")
	wireMust(t, err)
	theirs := filesExpose(w.ctx, t, w.backend, &other, desk.ID(), port)
	conversationEqual(t, theirs.Number, uint64(1))
	changes := "demi expose renew 1 && demi expose remove 1 && demi expose renew 1; echo exit=$?; " +
		add + "; demi expose add 8080 --host nope; echo exit=$?"
	w.vendor.Respond(conversationShell(t, "t3", changes, 20000))
	w.vendor.Respond(conversationAnswer(t, []string{"changed"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	result = conversationToolResult(t, requests[len(requests)-1], "t3")
	filesContains(
		t,
		result,
		"Expose 1 expires in 60 minutes.\n",
		"Removed expose 1; its URL no longer works.\n",
		"expose renew: no expose 1\n",
		"Expires in 60 minutes (expose 3).\n",
		"host nope is not reachable from this conversation",
	)
	conversationEqual(t, strings.Count(result, "exit=1"), 2)
	answer, err = backendtest.ReadAnswer(w.ctx, filesExposeFetch(w.ctx, t, w.backend, first, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	conversationEqual(t, filesExposeState(w.ctx, t, w.backend, &other).Exposes, []exposeplugin.ExposeEntry{theirs})
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// filesExposeCloud assembles the shared conversation driver with the cloud group's manager fixture.
func filesExposeCloud(
	t *testing.T,
	configure func(*backendtest.Harness),
) (*hostScenario, *providertest.MockVendor, *backendtest.ConversationSocket) {
	t.Helper()
	// Builds and multiple Cloud boots share the package's hang deadline;
	// their total elapsed time is not part of the expose contract.
	ctx := t.Context()
	h, manager, err := backendtest.HostsHarness(ctx, t)
	wireMust(t, err)
	filesExposeConfig(t, h)
	built, err := backendtest.BuildPackage(ctx, t, "demi-file")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	if configure != nil {
		configure(h)
	}
	b, user, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	vendor := providertest.StartVendor(t)
	entry := conversationAnthropic(ctx, t, b, &user, vendor)
	conversationCreate(ctx, t, b, &user, filesConversation)
	conversationChoose(ctx, t, b, &user, filesConversation, entry, "claude-opus-4-8")
	return &hostScenario{t, ctx, h, b, user, manager}, vendor, conversationOpen(ctx, t, b, &user, filesConversation)
}

// TestExposeCloudCheckpointKeepsConnectionAndIdleStopEndsIt uses a real cloud checkpoint to preserve its public
// relay; idle shutdown ends it before saving.
func TestExposeCloudCheckpointKeepsConnectionAndIdleStopEndsIt(t *testing.T) {
	t.Parallel()
	window := 600 * time.Millisecond
	s, vendor, socket := filesExposeCloud(t, func(h *backendtest.Harness) {
		h.Config.Lifecycle.IdleWindow = window
		h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
		h.Config.Cloud.Sweep = 50 * time.Millisecond
		h.Config.Cloud.CheckpointInterval = 300 * time.Millisecond
	})
	gate, err := backendtest.FileGate(s.ctx, s.b.Backend, s.user.User.ID, webapi.ConversationID(filesConversation))
	wireMust(t, err)
	working, err := gate.Enter(s.ctx, gates.Demand)
	wireMust(t, err)
	defer working.Release()
	vendor.Respond(conversationShell(t, "t1", "true", 20000))
	vendor.Respond(conversationAnswer(t, []string{"awake"}, 1, 1))
	_, err = socket.Chat(s.ctx, "m1", "go")
	wireMust(t, err)
	device := s.theCloud()
	proceed := make(chan struct{}, 1)
	service, released := filesExposeService(t, proceed)
	entry := filesExpose(s.ctx, t, s.b, &s.user, device, filesExposePort(t, service))
	held := filesExposeHeld(s.ctx, t, s.b, entry)
	checkpoint := "checkpoint:" + string(device)
	before := s.manager.Count(checkpoint)
	wireMust(t, s.manager.WaitCount(s.ctx, checkpoint, before+1))
	answer, err := backendtest.ReadAnswer(s.ctx, filesExposeFetch(s.ctx, t, s.b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 200)
	conversationEqual(t, string(answer.Body), "hello")
	proceed <- struct{}{}
	still := make([]byte, 6)
	_, err = io.ReadFull(held, still)
	wireMust(t, err)
	conversationEqual(t, string(still), "still\n")
	conversationEqual(t, filesExposeState(s.ctx, t, s.b, &s.user).Exposes, []exposeplugin.ExposeEntry{entry})
	rested := time.Now()
	working.Release()
	filesExposeEnded(s.ctx, t, held, released)
	conversationEqual(t, len(filesExposeState(s.ctx, t, s.b, &s.user).Exposes), 0)
	stopped, err := s.manager.Arrival(s.ctx, "hibernate:"+string(device))
	wireMust(t, err)
	if stopped.Before(rested.Add(window)) {
		t.Fatalf("cloud stopped after %s", stopped.Sub(rested))
	}
	answer, err = backendtest.ReadAnswer(s.ctx, filesExposeFetch(s.ctx, t, s.b, entry, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	wireMust(t, socket.Close(s.ctx))
	wireMust(t, s.b.Close(s.ctx))
}

// TestExposeCloudDeathResetRediscoveryAndBackendStartupEndRecords uses three cloud boots to exercise death,
// reset, silent loss, and startup cleanup of old exposes.
func TestExposeCloudDeathResetRediscoveryAndBackendStartupEndRecords(t *testing.T) {
	t.Parallel()
	s, vendor, socket := filesExposeCloud(t, nil)
	service, released := filesExposeService(t)
	port := filesExposePort(t, service)
	vendor.Respond(conversationShell(t, "t1", "true", 20000))
	vendor.Respond(conversationAnswer(t, []string{"awake"}, 1, 1))
	_, err := socket.Chat(s.ctx, "m1", "go")
	wireMust(t, err)
	device := s.theCloud()
	dying := filesExpose(s.ctx, t, s.b, &s.user, device, port)
	held := filesExposeHeld(s.ctx, t, s.b, dying)
	wireMust(t, s.manager.Kill(s.ctx, device))
	filesExposeEnded(s.ctx, t, held, released)
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	conversationEqual(t, len(filesExposeState(s.ctx, t, s.b, &s.user).Exposes), 0)
	vendor.Respond(conversationShell(t, "t2", "true", 20000))
	vendor.Respond(conversationAnswer(t, []string{"awake again"}, 1, 1))
	_, err = socket.Chat(s.ctx, "m2", "go")
	wireMust(t, err)
	resetting := filesExpose(s.ctx, t, s.b, &s.user, device, port)
	held = filesExposeHeld(s.ctx, t, s.b, resetting)
	s.resetCloud(cloudReset)
	filesExposeEnded(s.ctx, t, held, released)
	s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	})
	conversationEqual(t, len(filesExposeState(s.ctx, t, s.b, &s.user).Exposes), 0)
	found := filesExpose(s.ctx, t, s.b, &s.user, device, port)
	wireMust(t, s.manager.StopQuietly(s.ctx, device))
	wireMust(t, s.b.UntilOnline(s.ctx, &s.user, device, false))
	conversationEqual(t, filesExposeState(s.ctx, t, s.b, &s.user).Exposes, []exposeplugin.ExposeEntry{found})
	answer, err := backendtest.ReadAnswer(s.ctx, filesExposeFetch(s.ctx, t, s.b, found, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 502)
	vendor.Respond(conversationShell(t, "t3", "true", 20000))
	vendor.Respond(conversationAnswer(t, []string{"booted"}, 1, 1))
	_, err = socket.Chat(s.ctx, "m3", "go")
	wireMust(t, err)
	conversationEqual(t, len(filesExposeState(s.ctx, t, s.b, &s.user).Exposes), 0)
	answer, err = backendtest.ReadAnswer(s.ctx, filesExposeFetch(s.ctx, t, s.b, found, "/hello"))
	wireMust(t, err)
	filesStatus(t, answer, 404)
	wireMust(t, socket.Close(s.ctx))
	wireMust(t, s.b.Close(s.ctx))
	now, err := s.h.Clock.Now().Millisecond()
	wireMust(t, err)
	db, err := s.h.ControlDatabase(s.ctx, t)
	wireMust(t, err)
	_, err = db.ExecContext(
		s.ctx,
		"INSERT INTO exposes (id,user_id,device_id,address,created_at,expires_at) "+
			"VALUES ('k7x2maqw4p3s6tavaw2y4z6aab',?,?,'127.0.0.1:1',?,?)",
		string(s.user.User.ID),
		string(device),
		now,
		now+3600000,
	)
	wireMust(t, err)
	b, err := s.h.Start(s.ctx, t)
	wireMust(t, err)
	var records int
	wireMust(t, db.QueryRowContext(s.ctx, "SELECT COUNT(*) FROM exposes").Scan(&records))
	conversationEqual(t, records, 0)
	wireMust(t, b.Close(s.ctx))
}

// TestExposeWebSocketCarriesMessagesAndCloseCodesBothWays uses a local WebSocket service to echo messages and
// record the visitor's close.
func TestExposeWebSocketCarriesMessagesAndCloseCodesBothWays(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	handshakes := make(chan http.Header, 2)
	closed := make(chan websocket.CloseError, 1)
	handlerErrors := make(chan error, 2)
	var handlers sync.WaitGroup
	service := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		handlers.Add(1)
		defer handlers.Done()
		headers := r.Header.Clone()
		headers.Set("Host", r.Host)
		handshakes <- headers
		socket, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			handlerErrors <- err
			return
		}
		defer func() { _ = socket.CloseNow() }() // The read loop observes the peer close; cleanup is idempotent.
		for {
			kind, data, err := socket.Read(ctx)
			if err != nil {
				var end websocket.CloseError
				if errors.As(err, &end) {
					closed <- end
				} else {
					handlerErrors <- err
				}
				return
			}
			if kind == websocket.MessageText && string(data) == "close" {
				if err := socket.Close(websocket.StatusCode(4002), "service done"); err != nil {
					handlerErrors <- err
				}
				return
			}
			if err := socket.Write(ctx, kind, data); err != nil {
				handlerErrors <- err
				return
			}
		}
	}))
	t.Cleanup(func() {
		service.Close()
		handlers.Wait()
	})
	entry := filesExpose(ctx, t, b, &s, laptop.ID(), filesExposePort(t, service))
	public, err := url.Parse(entry.URL)
	wireMust(t, err)
	pongs := make(chan string, 1)
	socket, switched, err := websocket.Dial(
		ctx,
		b.WSURL("/socket"),
		&websocket.DialOptions{
			Host:           public.Host,
			HTTPClient:     b.HTTP,
			OnPongReceived: func(_ context.Context, payload []byte) { pongs <- string(payload) },
		},
	)
	wireMust(t, err)
	t.Cleanup(func() { _ = socket.CloseNow() })
	conversationEqual(t, switched.StatusCode, 101)
	if switched.Header.Get("Sec-WebSocket-Accept") == "" {
		t.Fatal("no upgrade accept")
	}
	var handshake http.Header
	select {
	case handshake = <-handshakes:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	conversationEqual(t, handshake.Get("Host"), "127.0.0.1:"+filesExposePort(t, service))
	if handshake.Get("Sec-WebSocket-Key") == "" {
		t.Fatal("no visitor key")
	}
	for _, message := range []struct {
		kind websocket.MessageType
		data []byte
	}{{websocket.MessageText, []byte("hello")}, {websocket.MessageBinary, []byte{0, 1, 2, 255}}} {
		wireMust(t, socket.Write(ctx, message.kind, message.data))
		kind, data, err := socket.Read(ctx)
		wireMust(t, err)
		conversationEqual(t, kind, message.kind)
		conversationEqual(t, data, message.data)
	}
	// coder/websocket generates the ping payload; its public reader dispatches the pong callback.
	reading := make(chan error, 1)
	go func() {
		_, _, err := socket.Read(ctx)
		reading <- err
	}()
	wireMust(t, socket.Ping(ctx))
	select {
	case pong := <-pongs:
		conversationEqual(t, pong, "1")
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	wireMust(t, socket.Close(websocket.StatusCode(4001), "visitor done"))
	select {
	case end := <-closed:
		conversationEqual(t, int(end.Code), 4001)
		conversationEqual(t, end.Reason, "visitor done")
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	err = <-reading
	var visitorEnd websocket.CloseError
	if !errors.As(err, &visitorEnd) {
		t.Fatalf("visitor close: %v", err)
	}
	conversationEqual(t, int(visitorEnd.Code), 4001)
	conversationEqual(t, visitorEnd.Reason, "visitor done")
	second, _, err := websocket.Dial(
		ctx,
		b.WSURL("/socket"),
		&websocket.DialOptions{Host: public.Host, HTTPClient: b.HTTP},
	)
	wireMust(t, err)
	t.Cleanup(func() { _ = second.CloseNow() })
	select {
	case <-handshakes:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	wireMust(t, second.Write(ctx, websocket.MessageText, []byte("close")))
	_, _, err = second.Read(ctx)
	var serviceEnd websocket.CloseError
	if !errors.As(err, &serviceEnd) {
		t.Fatalf("service close: %v", err)
	}
	conversationEqual(t, int(serviceEnd.Code), 4002)
	conversationEqual(t, serviceEnd.Reason, "service done")
	handlers.Wait()
	select {
	case err := <-handlerErrors:
		t.Fatal(err)
	default:
	}
	wireMust(t, b.Close(ctx))
}

// filesExposeHead preserves header spelling and order, which net/http normalizes.
// The connection's deadline is the owning scenario's context deadline.
func filesExposeHead(ctx context.Context, read *bufio.Reader) (string, error) {
	var head strings.Builder
	for {
		if err := ctx.Err(); err != nil {
			return "", err
		}
		line, err := read.ReadString('\n')
		if err != nil {
			return "", err
		}
		head.WriteString(line)
		if line == "\r\n" {
			return head.String(), nil
		}
	}
}

// filesExposeLines checks every occurrence of a relayed header, retaining its
// spelling and order instead of accepting an extra or differently cased line.
func filesExposeLines(t *testing.T, head, name string, want ...string) {
	t.Helper()
	var got []string
	for _, line := range strings.Split(head, "\r\n")[1:] {
		key, _, ok := strings.Cut(line, ":")
		if ok && strings.EqualFold(key, name) {
			got = append(got, line)
		}
	}
	conversationEqual(t, got, want)
}

// TestExposeRelayPreservesRequestsAnswersAndStreaming uses four raw HTTP exchanges to preserve headers,
// eight-MiB bodies, event streaming and refusals.
func TestExposeRelayPreservesRequestsAnswersAndStreaming(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	filesExposeConfig(t, h)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	wireMust(t, err)
	deadline, _ := ctx.Deadline()
	type observation struct {
		head        string
		body        []byte
		open, ended bool
	}
	seen := make(chan observation, 4)
	proceed := make(chan struct{})
	done := make(chan error, 1)
	var mu sync.Mutex
	var current net.Conn
	// The fixture owns its accept loop and current socket; failure cleanup unblocks and joins both.
	t.Cleanup(func() {
		_ = listener.Close()
		mu.Lock()
		active := current
		mu.Unlock()
		if active != nil {
			_ = active.Close()
		}
		if err := <-done; err != nil && !t.Failed() {
			t.Error(err)
		}
	})
	go func() {
		var failure error
		defer func() { done <- failure }()
		for range 4 {
			conn, err := listener.Accept()
			if err != nil {
				failure = err
				return
			}
			mu.Lock()
			current = conn
			mu.Unlock()
			failure = func() error {
				defer func() { _ = conn.Close() }() // The exchange checks I/O errors; cleanup may follow relay closure.
				if err := conn.SetDeadline(deadline); err != nil {
					return err
				}
				read := bufio.NewReader(conn)
				head, err := filesExposeHead(ctx, read)
				if err != nil {
					return err
				}
				switch {
				case strings.HasPrefix(head, "GET /headers"):
					seen <- observation{head: head}
					_, err = io.WriteString(
						conn,
						"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Service-Header: Yes\r\n"+
							"Set-Cookie: first=1; Path=/\r\nset-cookie: second=2; HttpOnly\r\nContent-Length: 5\r\n"+
							"Connection: close\r\n\r\nhello",
					)
					return err
				case strings.HasPrefix(head, "POST /upload"):
					// Read the trailers too: closing with an unread final CRLF can
					// reset TCP while the response is still reaching the runner.
					request, err := http.ReadRequest(bufio.NewReader(io.MultiReader(strings.NewReader(head), read)))
					if err != nil {
						return err
					}
					body, readErr := io.ReadAll(request.Body)
					if err := errors.Join(readErr, request.Body.Close()); err != nil {
						return err
					}
					seen <- observation{head: head, body: body}
					if _, err = io.WriteString(
						conn,
						"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
					); err != nil {
						return err
					}
					chunks := httputil.NewChunkedWriter(conn)
					if _, err = chunks.Write(backendtest.Pattern(8<<20, 7)); err != nil {
						return err
					}
					if err = chunks.Close(); err != nil {
						return err
					}
					_, err = io.WriteString(conn, "\r\n")
					return err
				case strings.HasPrefix(head, "GET /events"):
					inputEnd := make(chan struct{})
					go func() {
						_, _ = read.ReadByte()
						close(inputEnd)
					}()
					defer func() {
						_ = conn.Close()
						<-inputEnd
					}()
					if _, err = io.WriteString(
						conn,
						"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n"+
							"Transfer-Encoding: chunked\r\n\r\n",
					); err != nil {
						return err
					}
					chunks := httputil.NewChunkedWriter(conn)
					if _, err = io.WriteString(chunks, "event: tick\ndata: 1\n\n"); err != nil {
						return err
					}
					select {
					case <-proceed:
					case <-ctx.Done():
						return ctx.Err()
					}
					open := true
					select {
					case <-inputEnd:
						open = false
					default:
					}
					if _, err = io.WriteString(chunks, "event: tick\ndata: 2\n\n"); err != nil {
						return err
					}
					if err = chunks.Close(); err != nil {
						return err
					}
					if _, err = io.WriteString(conn, "\r\n"); err != nil {
						return err
					}
					select {
					case <-inputEnd:
						seen <- observation{open: open, ended: true}
						return nil
					case <-ctx.Done():
						return ctx.Err()
					}
				case strings.HasPrefix(head, "GET /refuse"):
					seen <- observation{head: head}
					_, err = io.WriteString(
						conn,
						"HTTP/1.1 426 Upgrade Required\r\nContent-Type: text/plain\r\nContent-Length: 17\r\n"+
							"Connection: close\r\n\r\nno upgrades here\n",
					)
					return err
				default:
					return fmt.Errorf("unexpected service request: %s", head)
				}
			}()
			if failure != nil {
				return
			}
		}
	}()
	port := strconv.Itoa(listener.Addr().(*net.TCPAddr).Port)
	entry := filesExpose(ctx, t, b, &s, laptop.ID(), port)
	public, err := url.Parse(entry.URL)
	wireMust(t, err)
	visit := func(request string) (net.Conn, *bufio.Reader, string) {
		t.Helper()
		conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", b.Address().String())
		wireMust(t, err)
		t.Cleanup(func() { _ = conn.Close() })
		wireMust(t, conn.SetDeadline(deadline))
		_, err = io.WriteString(conn, request)
		wireMust(t, err)
		read := bufio.NewReader(conn)
		head, err := filesExposeHead(ctx, read)
		wireMust(t, err)
		return conn, read, head
	}
	observe := func() observation {
		select {
		case v := <-seen:
			return v
		case <-ctx.Done():
			t.Fatal(ctx.Err())
			return observation{}
		}
	}
	_, read, answer := visit(
		"GET /headers?q=1 HTTP/1.1\r\nHost: " + public.Host + "\r\n" +
			"X-Custom-Header: One\r\nx-lower-case: two\r\nX-UPPER-CASE: THREE\r\nCookie: a=1; b=2\r\n" +
			"Connection: keep-alive, X-Hop\r\nKeep-Alive: timeout=5\r\nX-Hop: gone\r\nX-Custom-Header: Four\r\n\r\n",
	)
	observed := observe()
	if !strings.HasPrefix(observed.head, "GET /headers?q=1 HTTP/1.1\r\n") {
		t.Fatal(observed.head)
	}
	for _, line := range []string{
		"Host: 127.0.0.1:" + port,
		"x-lower-case: two",
		"X-UPPER-CASE: THREE",
		"Cookie: a=1; b=2",
	} {
		name, _, _ := strings.Cut(line, ":")
		filesExposeLines(t, observed.head, name, line)
	}
	filesExposeLines(t, observed.head, "x-custom-header", "X-Custom-Header: One", "X-Custom-Header: Four")
	for _, line := range []string{
		"connection: close",
		"x-forwarded-for: 127.0.0.1",
		"x-forwarded-host: " + public.Host,
		"x-forwarded-proto: http",
	} {
		name, _, _ := strings.Cut(line, ":")
		filesExposeLines(t, strings.ToLower(observed.head), name, line)
	}
	conversationEqual(t, len(strings.Split(strings.TrimSuffix(observed.head, "\r\n\r\n"), "\r\n"))-1, 10)
	conversationEqual(t, strings.SplitN(answer, "\r\n", 2)[0], "HTTP/1.1 200 OK")
	for _, line := range []string{"Content-Type: text/plain", "X-Service-Header: Yes", "Content-Length: 5"} {
		name, _, _ := strings.Cut(line, ":")
		filesExposeLines(t, answer, name, line)
	}
	filesExposeLines(t, answer, "set-cookie", "Set-Cookie: first=1; Path=/", "set-cookie: second=2; HttpOnly")
	filesExposeLines(t, answer, "connection")
	body := make([]byte, 5)
	_, err = io.ReadFull(read, body)
	wireMust(t, err)
	conversationEqual(t, string(body), "hello")
	// Upload and response use the standard library's chunk framing over raw sockets.
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", b.Address().String())
	wireMust(t, err)
	t.Cleanup(func() { _ = conn.Close() })
	wireMust(t, conn.SetDeadline(deadline))
	_, err = io.WriteString(
		conn,
		"POST /upload HTTP/1.1\r\nHost: "+public.Host+"\r\n"+
			"Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
	)
	wireMust(t, err)
	chunks := httputil.NewChunkedWriter(conn)
	_, err = chunks.Write(backendtest.Pattern(8<<20, 3))
	wireMust(t, err)
	wireMust(t, chunks.Close())
	_, err = io.WriteString(conn, "\r\n")
	wireMust(t, err)
	read = bufio.NewReader(conn)
	answer, err = filesExposeHead(ctx, read)
	wireMust(t, err)
	conversationEqual(t, strings.SplitN(answer, "\r\n", 2)[0], "HTTP/1.1 200 OK")
	filesExposeLines(t, strings.ToLower(answer), "transfer-encoding", "transfer-encoding: chunked")
	body, err = io.ReadAll(httputil.NewChunkedReader(read))
	wireMust(t, err)
	if !bytes.Equal(body, backendtest.Pattern(8<<20, 7)) {
		t.Fatal("response body changed")
	}
	observed = observe()
	conversationEqual(t, strings.SplitN(observed.head, "\r\n", 2)[0], "POST /upload HTTP/1.1")
	filesExposeLines(t, strings.ToLower(observed.head), "transfer-encoding", "transfer-encoding: chunked")
	if !bytes.Equal(observed.body, backendtest.Pattern(8<<20, 3)) {
		t.Fatal("request body changed")
	}
	_, read, answer = visit("GET /events HTTP/1.1\r\nHost: " + public.Host + "\r\nAccept: text/event-stream\r\n\r\n")
	conversationEqual(t, strings.SplitN(answer, "\r\n", 2)[0], "HTTP/1.1 200 OK")
	filesExposeLines(t, answer, "Content-Type", "Content-Type: text/event-stream")
	events := httputil.NewChunkedReader(read)
	for i := 1; i <= 2; i++ {
		want := fmt.Sprintf("event: tick\ndata: %d\n\n", i)
		event := make([]byte, len(want))
		_, err = io.ReadFull(events, event)
		wireMust(t, err)
		conversationEqual(t, string(event), want)
		if i == 1 {
			close(proceed)
		}
	}
	tail, err := io.ReadAll(events)
	wireMust(t, err)
	conversationEqual(t, len(tail), 0)
	observed = observe()
	conversationEqual(t, observed.open, true)
	conversationEqual(t, observed.ended, true)
	_, read, answer = visit(
		"GET /refuse HTTP/1.1\r\nHost: " + public.Host + "\r\n" +
			"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n" +
			"Sec-WebSocket-Version: 13\r\n\r\n",
	)
	conversationEqual(t, strings.SplitN(answer, "\r\n", 2)[0], "HTTP/1.1 426 Upgrade Required")
	filesExposeLines(t, answer, "Content-Length", "Content-Length: 17")
	body = make([]byte, 17)
	_, err = io.ReadFull(read, body)
	wireMust(t, err)
	conversationEqual(t, string(body), "no upgrades here\n")
	observed = observe()
	for _, line := range []string{
		"Upgrade: websocket",
		"Connection: Upgrade",
		"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
		"Sec-WebSocket-Version: 13",
	} {
		name, _, _ := strings.Cut(line, ":")
		filesExposeLines(t, observed.head, name, line)
	}
	wireMust(t, b.Close(ctx))
}

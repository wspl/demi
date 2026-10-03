package backend_test

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// rawHostRunner speaks only the runner protocol, leaving backend admission real.
type rawHostRunner struct {
	t      *testing.T
	ctx    context.Context
	socket *websocket.Conn
}

func connectHostRunner(t *testing.T, b *backendtest.TestBackend) *rawHostRunner {
	t.Helper()
	socket, response, err := websocket.Dial(t.Context(), b.WSURL("/api/runner"), nil)
	if response != nil && response.Body != nil {
		_ = response.Body.Close()
	}
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = socket.CloseNow() }) // Closing an already closed fixture socket is harmless.
	return &rawHostRunner{t, t.Context(), socket}
}
func (r *rawHostRunner) send(message runnerwire.Outbound) {
	r.t.Helper()
	data, err := runnerwire.Encode(message)
	if err != nil {
		r.t.Fatal(err)
	}
	if err := r.socket.Write(r.ctx, websocket.MessageBinary, data); err != nil {
		r.t.Fatal(err)
	}
}
func (r *rawHostRunner) next() runnerwire.Inbound {
	r.t.Helper()
	ctx, cancel := context.WithTimeout(r.ctx, 10*time.Second)
	defer cancel()
	kind, data, err := r.socket.Read(ctx)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) {
			r.t.Fatal("runner answer timed out")
		}
		return nil
	}
	if kind != websocket.MessageBinary {
		r.t.Fatalf("unexpected runner frame: %v", kind)
	}
	message, err := runnerwire.DecodeInbound(data)
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}
func runnerHello(protocol uint32, token string, managed *bool) *runnerwire.Hello {
	h := &runnerwire.Hello{Protocol: protocol, Runner: runnerwire.RunnerInfo{Name: "raw", Platform: runnerwire.RunnerPlatformLinux, Version: "0", Identity: runnerwire.HostIdentity{UID: 1, GID: 1, Hostname: "raw", HomeDir: "/home/raw"}, Managed: managed}}
	if token != "" {
		v := runnerwire.DeviceToken(token)
		h.DeviceToken = &v
	}
	return h
}
func (s *hostScenario) stoppedPair() (*backendtest.Paired, string) {
	s.t.Helper()
	p := s.pair("laptop")
	token, err := p.Token(s.ctx)
	if err != nil {
		s.t.Fatal(err)
	}
	if err := p.Runner.Stop(s.ctx); err != nil {
		s.t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, p.ID(), false); err != nil {
		s.t.Fatal(err)
	}
	return p, token
}

func TestRunnerRejectsInvalidProtocolAndUnknownIdentity(t *testing.T) {
	h, _, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Runners.HelloDeadline = 300 * time.Millisecond
	b, err := h.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		kind websocket.MessageType
		data []byte
	}{
		{websocket.MessageText, []byte(`{"type":"not-a-runner-frame"}`)},
		{websocket.MessageBinary, []byte{0xc1, 0}},
	} {
		r := connectHostRunner(t, b)
		if err := r.socket.Write(t.Context(), tc.kind, tc.data); err != nil {
			t.Fatal(err)
		}
		if got := r.next(); got != nil {
			t.Fatalf("invalid frame answered: %v", got)
		}
	}
	managed := true
	for _, tc := range []struct {
		hello *runnerwire.Hello
		code  runnerwire.HelloErrorCode
		words string
	}{
		{runnerHello(runnerwire.Version, "not-a-real-token", nil), runnerwire.HelloErrorCodeUnknownDevice, "unknown device"},
		{runnerHello(runnerwire.Version+1, "", nil), runnerwire.HelloErrorCodeUnsupportedProtocol, "unsupported protocol"},
		{runnerHello(runnerwire.Version, "", &managed), runnerwire.HelloErrorCodeUnknownDevice, "never paired"},
	} {
		r := connectHostRunner(t, b)
		r.send(tc.hello)
		refused, ok := r.next().(*runnerwire.HelloError)
		if !ok || refused.Code != tc.code || !strings.Contains(refused.Reason, tc.words) {
			t.Fatalf("hello refusal: %+v", refused)
		}
		if r.next() != nil {
			t.Fatal("refused socket remained open")
		}
	}
	silent := connectHostRunner(t, b)
	started := time.Now()
	if silent.next() != nil {
		t.Fatal("silent connection answered")
	}
	if time.Since(started) < 250*time.Millisecond {
		t.Fatal("hello deadline closed early")
	}
}

func TestConcurrentHellosBindOnceAndRepeatedHelloIsIgnored(t *testing.T) {
	s := newHostScenario(t, "")
	laptop, token := s.stoppedPair()
	one, other := connectHostRunner(t, s.b), connectHostRunner(t, s.b)
	hello := runnerHello(runnerwire.Version, token, nil)
	var sends sync.WaitGroup
	sends.Go(func() { one.send(hello) })
	sends.Go(func() { other.send(hello) })
	sends.Wait()
	first, second := one.next(), other.next()
	bound, refused := one, second
	if _, ok := first.(*runnerwire.HelloOK); !ok {
		bound, refused = other, first
		if _, ok := second.(*runnerwire.HelloOK); !ok {
			t.Fatalf("neither welcomed: %v %v", first, second)
		}
	}
	refusal, ok := refused.(*runnerwire.HelloError)
	if !ok || refusal.Code != runnerwire.HelloErrorCodeAlreadyConnected {
		t.Fatalf("other: %+v", refused)
	}
	online, err := s.b.Online(s.ctx, &s.user, laptop.ID())
	if err != nil || !online {
		t.Fatalf("online: %v %v", online, err)
	}
	bound.send(hello)
	bound.send(&runnerwire.Pong{Jobs: 0})
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	bound.send(&runnerwire.ArtifactResolve{ID: "after-hello", Owner: &runnerwire.StreamArtifactOwner{StreamID: "none"}, SHA256: strings.Repeat("0", 64), Target: string(target)})
	answer, ok := bound.next().(*runnerwire.ArtifactLocation)
	if !ok || answer.ID != "after-hello" || answer.Error == nil {
		t.Fatalf("later request: %+v", answer)
	}
	online, err = s.b.Online(s.ctx, &s.user, laptop.ID())
	if err != nil || !online {
		t.Fatalf("online: %v %v", online, err)
	}
	_ = bound.socket.CloseNow()
	if err := s.b.UntilOnline(s.ctx, &s.user, laptop.ID(), false); err != nil {
		t.Fatal(err)
	}
}

func TestRunnerReservesOnlyReachableConversationNumbers(t *testing.T) {
	s := newHostScenario(t, "")
	laptop := s.pair("laptop")
	token, err := laptop.Token(s.ctx)
	if err != nil {
		t.Fatal(err)
	}
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	s.move(laptop, laptop.Runner.Home())
	const elsewhere = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a02"
	conversationCreate(s.ctx, s.t, s.b, &s.user, elsewhere)
	if err := laptop.Runner.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, laptop.ID(), false); err != nil {
		t.Fatal(err)
	}
	r := connectHostRunner(t, s.b)
	r.send(runnerHello(runnerwire.Version, token, nil))
	if _, ok := r.next().(*runnerwire.HelloOK); !ok {
		t.Fatal("not welcomed")
	}
	for i, tc := range []struct {
		conversation string
		count        uint32
		first        uint64
	}{
		{hostsConversation, 4, 1}, {hostsConversation, 1, 5}, {elsewhere, 1, 0}, {"no-such-conversation", 1, 0}, {hostsConversation, 2, 6},
	} {
		id := fmt.Sprint(i)
		r.send(&runnerwire.NumbersReserve{ID: id, ConversationID: tc.conversation, Sequence: commandwire.TabSequence, Count: tc.count})
		for {
			message := r.next()
			if message == nil {
				t.Fatal("connection ended")
			}
			answer, ok := message.(*runnerwire.NumbersReserved)
			if !ok || answer.ID != id {
				continue
			}
			if tc.first == 0 {
				if answer.First != nil || answer.Error == nil {
					t.Fatalf("refusal: %+v", answer)
				}
			} else if answer.First == nil || *answer.First != tc.first || answer.Error != nil {
				t.Fatalf("reservation: %+v", answer)
			}
			break
		}
	}
}

func TestDisconnectedRunnerCancelsHeldTokenLookup(t *testing.T) {
	s := newHostScenario(t, "")
	_, token := s.stoppedPair()
	hold := backendtest.HoldHellos(t, s.b.Backend, backendtest.HelloTokenLookup)
	r := connectHostRunner(t, s.b)
	r.send(runnerHello(runnerwire.Version, token, nil))
	if err := hold.UntilArrived(s.ctx, 1); err != nil {
		t.Fatal(err)
	}
	// Close sends the close frame and waits for the peer, as Rust next() does.
	// EOF is an ended peer; a close-handshake timeout is not.
	if err := r.socket.Close(websocket.StatusNormalClosure, ""); errors.Is(err, context.DeadlineExceeded) {
		t.Fatal(err)
	}
	if r.next() != nil {
		t.Fatal("disconnected runner answered")
	}
	hold.Release()
}

func TestClaimOfDisconnectedRunnerCreatesNoDevice(t *testing.T) {
	s := newHostScenario(t, "")
	r := connectHostRunner(t, s.b)
	r.send(runnerHello(runnerwire.Version, "", nil))
	pending, ok := r.next().(*runnerwire.ClaimPending)
	if !ok {
		t.Fatal("no pairing code")
	}
	if err := r.socket.CloseNow(); err != nil {
		t.Fatal(err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, pending.ClaimToken), 404), webapi.ErrorCodeInvalidCode)
	devices, err := s.b.Devices(s.ctx, &s.user)
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 0 {
		t.Fatal(devices)
	}
}

func TestPipesRequireDeviceToken(t *testing.T) {
	s := newHostScenario(t, "")
	laptop := s.pair("laptop")
	token, err := laptop.Token(s.ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, method := range []string{"GET", "PUT"} {
		for _, tc := range []struct {
			headers http.Header
			status  int
		}{
			{nil, 401}, {http.Header{"Authorization": {"Bearer not-a-token"}}, 401}, {http.Header{"Cookie": {s.user.Cookie}}, 401}, {http.Header{"Authorization": {"Bearer " + token}}, 404},
		} {
			response, err := s.b.Response(s.ctx, method, "/api/pipes/0123456789abcdef", nil, tc.headers, strings.NewReader("bytes"))
			if err != nil {
				t.Fatal(err)
			}
			a, err := backendtest.ReadAnswer(s.ctx, response)
			if err != nil {
				t.Fatal(err)
			}
			if a.Status != tc.status {
				t.Fatalf("pipe: %d %s", a.Status, a.Body)
			}
			if tc.headers == nil && string(a.Body) != "device token required" {
				t.Fatal(string(a.Body))
			}
		}
	}
}

func TestDevicesAndRunnersReturnAfterBackendRestart(t *testing.T) {
	s := newHostScenario(t, "")
	laptop := s.pair("laptop")
	if err := s.h.Clock.Advance(time.Second); err != nil {
		t.Fatal(err)
	}
	desktop := s.pair("desktop")
	address := s.b.Address()
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	s.b, err = s.h.StartAt(s.ctx, t, address)
	if err != nil {
		t.Fatal(err)
	}
	for _, p := range []*backendtest.Paired{laptop, desktop} {
		if err := s.b.UntilOnline(s.ctx, &s.user, p.ID(), true); err != nil {
			t.Fatal(err)
		}
	}
	devices, err := s.b.Devices(s.ctx, &s.user)
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 2 || devices[0].Name != "laptop" || devices[1].Name != "desktop" {
		t.Fatal(devices)
	}
	for _, d := range devices {
		if d.LastSeenAt == nil {
			t.Fatal("missing last seen")
		}
	}
}

func TestRunnerHelloDuringShutdownIsNotWelcomed(t *testing.T) {
	s := newHostScenario(t, "")
	_, token := s.stoppedPair()
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	page, err := s.b.Conversation(s.ctx, t, &s.user, hostsConversation)
	if err != nil {
		t.Fatal(err)
	}
	wireMust(t, page.Send(s.ctx, &framewire.AbortFrame{}))
	_, err = page.Next(s.ctx)
	wireMust(t, err)
	hold := backendtest.HoldHellos(t, s.b.Backend, backendtest.HelloBind)
	runner := connectHostRunner(t, s.b)
	runner.send(runnerHello(runnerwire.Version, token, nil))
	if err := hold.UntilArrived(s.ctx, 1); err != nil {
		t.Fatal(err)
	}
	closed := make(chan error, 1)
	go func() { closed <- s.b.Close(s.ctx) }()
	defer func() {
		hold.Release()
		_ = runner.socket.CloseNow() // Release the owned socket even if an assertion failed.
		if err := <-closed; err != nil {
			t.Error(err)
		}
	}()
	code, _, err := page.Closed(s.ctx)
	wireMust(t, err)
	if code != websocket.StatusGoingAway {
		t.Fatalf("page close: %v", code)
	}
	hold.Release()
	if runner.next() != nil {
		t.Fatal("runner welcomed during shutdown")
	}
}

// A code expiry wakes the runner's output wait; no test sleep polls the code.
func TestPairingCodeExpiresAndClaimAttemptsAreLimited(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Runners.ClaimLifetime = time.Second
	h.Config.Runners.ClaimsPerMinute = 3
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	runner, err := remotehosttest.StartRunnerProcess(s.ctx, t, b.URL, remotehosttest.DefaultRunnerProcessOptions())
	if err != nil {
		t.Fatal(err)
	}
	first, err := runner.PairingCode(s.ctx, 0)
	if err != nil {
		t.Fatal(err)
	}
	second, err := runner.PairingCode(s.ctx, 1)
	if err != nil {
		t.Fatal(err)
	}
	if first == second {
		t.Fatal("pairing code did not change")
	}
	claimed := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, second), 201)
	device, err := webapi.DecodeClaimedDevice(claimed.Body)
	if err != nil {
		t.Fatal(err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, first), 404), webapi.ErrorCodeInvalidCode)
	if err := b.UntilOnline(s.ctx, &user, device.Device.ID, true); err != nil {
		t.Fatal(err)
	}
	invalid := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", `{"code":""}`, 400)
	errorBody, err := invalid.ErrorBody()
	if err != nil {
		t.Fatal(err)
	}
	if errorBody.Code != webapi.ErrorCodeInvalidBody {
		t.Fatal(errorBody)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", `{"code":"NOPE-NOPE"}`, 404), webapi.ErrorCodeInvalidCode)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", `{"code":"NOPE-NOPE"}`, 429), webapi.ErrorCodeRateLimited)
}

func (s *hostScenario) deviceLog(path string) webapi.DeviceLog {
	s.t.Helper()
	read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", path, "", 200)
	log, err := webapi.DecodeDeviceLog(read.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return log
}
func TestPairedDeviceDirectoryAndLogAccess(t *testing.T) {
	s := newHostScenario(t, "")
	laptop := s.pair("laptop")
	home := laptop.Runner.Home()
	if err := os.WriteFile(filepath.Join(home, "hello.txt"), []byte("hi"), 0644); err != nil {
		t.Fatal(err)
	}
	fs := "/api/devices/" + string(laptop.ID()) + "/fs"
	listing := s.directoryListing(fs)
	if listing.Path != home || listing.Home == nil || *listing.Home != home {
		t.Fatal(listing)
	}
	found := false
	for _, entry := range listing.Entries {
		if entry.Name == "hello.txt" {
			found = true
			if entry.IsDirectory || entry.IsSymbolicLink || entry.Size != 2 {
				t.Fatal(entry)
			}
		}
	}
	if !found {
		t.Fatal("hello.txt not listed")
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", fs, fmt.Sprintf(`{"path":%q}`, home+"/made/by/web"), 201)
	info, err := os.Stat(filepath.Join(home, "made/by/web"))
	if err != nil || !info.IsDir() {
		t.Fatalf("created directory: %v", err)
	}
	for _, query := range []string{"?path=relative", "?path="} {
		conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", fs+query, "", 400), webapi.ErrorCodeInvalidQuery)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", fs+"?path="+url.QueryEscape(home+"/nothing"), "", 404), webapi.ErrorCodeFsError)
	path := "/api/devices/" + string(laptop.ID()) + "/log"
	tail := s.deviceLog(path)
	online, pairing := false, false
	code, err := laptop.Runner.PairingCode(s.ctx, 0)
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range tail.Lines {
		online = online || line.Text == "online"
		pairing = pairing || line.Text == "waiting to be paired"
		if line.Source != "runner" || strings.Contains(line.Text, code) {
			t.Fatal(line)
		}
	}
	if !online || !pairing {
		t.Fatal(tail)
	}
	after := s.deviceLog(fmt.Sprintf("%s?since=%d", path, tail.Next))
	if len(after.Lines) != 0 || after.Next != tail.Next {
		t.Fatal(after)
	}
	first := s.deviceLog(path + "?since=0&limit=1")
	if diff := cmp.Diff(tail.Lines[:1], first.Lines); diff != "" {
		t.Fatal(diff)
	}
	second := s.deviceLog(fmt.Sprintf("%s?since=%d&limit=1", path, first.Next))
	if diff := cmp.Diff(tail.Lines[1:2], second.Lines); diff != "" {
		t.Fatal(diff)
	}
	other := s.deviceLog(path + "?since=0&source=service%3Anone")
	if len(other.Lines) != 0 || other.Next != tail.Next {
		t.Fatal(other)
	}
	for _, query := range []string{"limit=0", "limit=1001", "limit=many", "since=-1", "since=1.5", "source="} {
		conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", path+"?"+query, "", 400), webapi.ErrorCodeInvalidQuery)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/devices/none/log", "", 404), webapi.ErrorCodeDeviceNotFound)
	if err := laptop.Runner.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, laptop.ID(), false); err != nil {
		t.Fatal(err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", path, "", 409), webapi.ErrorCodeDeviceOffline)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", fs, "", 409), webapi.ErrorCodeDeviceOffline)
}

// A real runner persists its token across restart and exits when revoked.
func TestClaimedRunnerReconnectsUntilRevoked(t *testing.T) {
	s := newHostScenario(t, "")
	runner, err := remotehosttest.StartRunnerProcess(s.ctx, t, s.b.URL, remotehosttest.RunnerProcessOptions{Name: "laptop"})
	if err != nil {
		t.Fatal(err)
	}
	code, err := runner.PairingCode(s.ctx, 0)
	if err != nil {
		t.Fatal(err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", `{"code":"AAAA-BBBB"}`, 404), webapi.ErrorCodeInvalidCode)
	answer := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, " "+strings.ToLower(code)+" "), 201)
	claimed, err := webapi.DecodeClaimedDevice(answer.Body)
	if err != nil {
		t.Fatal(err)
	}
	device := claimed.Device
	if device.Name != "laptop" || device.Kind != webapi.DeviceKindUser || !device.Online || device.Home == nil || *device.Home != runner.Home() {
		t.Fatalf("device: %+v", device)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, code), 404), webapi.ErrorCodeInvalidCode)
	listed, err := s.b.Devices(s.ctx, &s.user)
	if err != nil {
		t.Fatal(err)
	}
	if len(listed) != 1 || listed[0].ID != device.ID || !listed[0].Online {
		t.Fatalf("devices: %+v", listed)
	}
	if _, err := backendtest.StoredToken(s.ctx, runner); err != nil {
		t.Fatal(err)
	}
	if err := runner.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, device.ID, false); err != nil {
		t.Fatal(err)
	}
	if err := runner.StartAgain(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, device.ID, true); err != nil {
		t.Fatal(err)
	}
	listed, err = s.b.Devices(s.ctx, &s.user)
	if err != nil || len(listed) != 1 {
		t.Fatalf("devices: %+v, %v", listed, err)
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+string(device.ID), "", 204)
	// Rust observes termination, without requiring a particular exit status.
	err = runner.Exited(s.ctx)
	if err != nil && s.ctx.Err() != nil {
		t.Fatal(err)
	}
	if !strings.Contains(runner.Output(), "revoked") {
		t.Fatal(runner.Output())
	}
	listed, err = s.b.Devices(s.ctx, &s.user)
	if err != nil || len(listed) != 0 {
		t.Fatalf("devices: %+v, %v", listed, err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+string(device.ID), "", 404), webapi.ErrorCodeDeviceNotFound)
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
}

// The twin's real reconnect backoff makes this scenario take several seconds.
func TestRunnerTwinIsAdoptedAfterFirstDisconnects(t *testing.T) {
	s := newHostScenario(t, "")
	first := s.pair("laptop")
	token, err := first.Token(s.ctx)
	if err != nil {
		t.Fatal(err)
	}
	twin, err := remotehosttest.StartRunnerProcess(s.ctx, t, s.b.URL, remotehosttest.RunnerProcessOptions{Name: "laptop", Token: &token})
	if err != nil {
		t.Fatal(err)
	}
	if err := twin.UntilOutput(s.ctx, "already_connected"); err != nil {
		t.Fatal(err)
	}
	online, err := s.b.Online(s.ctx, &s.user, first.ID())
	if err != nil || !online || !twin.Running() {
		t.Fatalf("online: %v, twin running: %v, %v", online, twin.Running(), err)
	}
	if err := first.Runner.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, first.ID(), true); err != nil {
		t.Fatal(err)
	}
	if !twin.Running() {
		t.Fatal(twin.Output())
	}
	if err := twin.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, first.ID(), false); err != nil {
		t.Fatal(err)
	}
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
}

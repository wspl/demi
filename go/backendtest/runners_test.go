package backendtest_test

import (
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/controlproto"
	"github.com/wspl/demi/go/backendtest/runnerproc"
)

// Cost: one backend and a real runner, about a second.
func TestAClaimedRunnerReconnectsWithItsTokenUntilItsDeviceIsRevoked(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	runner := b.StartRunner(runnerproc.Options{Name: "laptop"})
	code := backendtest.PairingCode(t, runner, 0)
	claim := func(code string) *backendtest.Answer {
		return b.Post("/api/devices/claim", master, backendtest.Map{"code": code})
	}

	wantRefusal(t, claim("AAAA-BBBB"), http.StatusNotFound, "invalid_code", "a wrong code")
	// Entered messy, the code still names the runner.
	claimed := claim(" " + strings.ToLower(code) + " ").Expect(http.StatusCreated)
	device := claimed.At("device").(map[string]any)
	if device["name"] != "laptop" || device["kind"] != "user" || device["online"] != true || device["home"] != runner.Home() {
		t.Fatalf("the claimed device is %v", device)
	}
	// A code is single use.
	wantRefusal(t, claim(code), http.StatusNotFound, "invalid_code", "a used code")
	listed := b.Devices(master)
	if len(listed) != 1 || listed[0]["id"] != device["id"] || listed[0]["online"] != true {
		t.Fatalf("the devices are %v", listed)
	}

	// A restarted runner presents the token it stored and is the same device,
	// online.
	backendtest.StoredToken(t, runner)
	runner.Stop()
	id := device["id"].(string)
	b.UntilOnline(master, id, false)
	if err := runner.StartAgain(); err != nil {
		t.Fatal(err)
	}
	b.UntilOnline(master, id, true)
	if got := len(b.Devices(master)); got != 1 {
		t.Fatalf("the restarted runner is %d devices", got)
	}

	// Revoked, the device is gone, and its runner hears why and stops.
	b.Delete("/api/devices/"+id, master).Expect(http.StatusNoContent)
	backendtest.Eventually(t, "the revoked runner stops", func() bool { return !runner.Running() })
	if !strings.Contains(runner.Output(), "revoked") {
		t.Fatalf("the runner does not say why it stopped: %s", runner.Output())
	}
	if got := b.Devices(master); len(got) != 0 {
		t.Fatalf("the revoked device is listed: %v", got)
	}
	wantRefusal(t, b.Delete("/api/devices/"+id, master), http.StatusNotFound, "device_not_found", "revoking a revoked device")
	b.Stop()
}

// Cost: one backend and a real runner, over a second: a pairing code's lifetime
// of one second passes in real time.
func TestAWaitingRunnersCodeChangesWhileItWaitsAndClaimsAreLimited(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	h.Runners().ClaimLifetimeMs = backendtest.Ptr(uint64(1000))
	h.Runners().ClaimsPerMinute = backendtest.Ptr(uint64(3))
	b, master := h.StartSetUp()
	runner := b.StartRunner(runnerproc.Options{Name: "fixture"})
	first := backendtest.PairingCode(t, runner, 0)
	second := backendtest.PairingCode(t, runner, 1)
	if first == second {
		t.Fatal("the code did not change")
	}
	claim := func(code string) *backendtest.Answer {
		return b.Post("/api/devices/claim", master, backendtest.Map{"code": code})
	}

	// The expired code is dead; the one the runner printed last claims it.
	claimed := claim(second).Expect(http.StatusCreated)
	wantRefusal(t, claim(first), http.StatusNotFound, "invalid_code", "the expired code")
	b.UntilOnline(master, claimed.Str("device.id"), true)

	// Three attempts a minute: the fourth is refused before any code is looked
	// at, and a body that names no code is no attempt.
	if _, code := claim("").Refusal(); code != "invalid_body" {
		t.Fatalf("an empty code is refused with %s", code)
	}
	wantRefusal(t, claim("NOPE-NOPE"), http.StatusNotFound, "invalid_code", "the first attempt")
	wantRefusal(t, claim("NOPE-NOPE"), http.StatusTooManyRequests, "rate_limited", "the limited attempt")
	b.Stop()
}

// helloRefusal reads the refusal of a runner's hello, and the close after it.
func helloRefusal(t *testing.T, runner *backendtest.RawRunner) (code, reason string) {
	t.Helper()
	message := runner.Next()
	if message == nil || message["type"] != "hello_error" {
		t.Fatalf("expected a refusal, got %v", message)
	}
	if next := runner.Next(); next != nil {
		t.Fatalf("the backend sends %v after the refusal", next)
	}
	code, _ = message["code"].(string)
	reason, _ = message["reason"].(string)
	return code, reason
}

// Cost: one backend, about a second: a connection that says nothing waits for
// the hello deadline of 0.3 s.
func TestARunnerThatBreaksTheProtocolOrNamesNoDeviceIsRefused(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	h.Runners().HelloDeadlineMs = backendtest.Ptr(uint64(300))
	b := h.Start()

	text := b.ConnectRawRunner()
	text.SendText(`{"type":"not-a-runner-frame"}`)
	if message := text.Next(); message != nil {
		t.Fatalf("a text frame is answered with %v", message)
	}
	garbage := b.ConnectRawRunner()
	garbage.SendBinary([]byte{0xc1, 0x00})
	if message := garbage.Next(); message != nil {
		t.Fatalf("a frame of garbage is answered with %v", message)
	}

	yes := true
	for _, refusal := range []struct {
		hello        map[string]any
		code, reason string
	}{
		{backendtest.RunnerHello(backendtest.RunnerVersion, "not-a-real-token", nil), "unknown_device", "unknown device"},
		{backendtest.RunnerHello(backendtest.RunnerVersion+1, "", nil), "unsupported_protocol", "unsupported protocol"},
		{backendtest.RunnerHello(backendtest.RunnerVersion, "", &yes), "unknown_device", "never paired"},
	} {
		runner := b.ConnectRawRunner()
		runner.Send(refusal.hello)
		code, reason := helloRefusal(t, runner)
		if code != refusal.code || !strings.Contains(reason, refusal.reason) {
			t.Fatalf("the hello is refused with %s %q, not %s %q", code, reason, refusal.code, refusal.reason)
		}
	}

	// A connection that says nothing is closed after the hello deadline.
	silent := b.ConnectRawRunner()
	started := time.Now()
	if message := silent.Next(); message != nil {
		t.Fatalf("a silent connection is answered with %v", message)
	}
	if elapsed := time.Since(started); elapsed < 250*time.Millisecond {
		t.Fatalf("the silent connection was closed after %s", elapsed)
	}
	b.Stop()
}

// Cost: one backend and two real runners, about a second.
func TestADeviceHoldsOneLiveConnectionAndANewcomerIsAdoptedOnceTheFirstIsGone(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	first := b.Pair(master, "laptop")
	token := first.Token()

	// The same token from a second process is refused while the first is
	// connected, and the first keeps its connection.
	twin := b.StartRunner(runnerproc.Options{Name: "laptop", Token: token})
	backendtest.Eventually(t, "the twin is refused", func() bool {
		return strings.Contains(twin.Output(), "already_connected")
	})
	if !b.Online(master, first.ID()) || !twin.Running() {
		t.Fatal("the twin's refusal changed the device or its runner")
	}

	// Once the first is gone, the twin's next attempt is adopted.
	first.Runner.Stop()
	b.UntilOnline(master, first.ID(), true)
	if !twin.Running() {
		t.Fatal("the twin stopped")
	}
	twin.Stop()
	b.UntilOnline(master, first.ID(), false)
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestHellosWithOneTokenAtOnceBindOneSocketAndARepeatedHelloChangesNothing(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	token := laptop.Token()
	laptop.Runner.Stop()
	b.UntilOnline(master, laptop.ID(), false)

	sockets := []*backendtest.RawRunner{b.ConnectRawRunner(), b.ConnectRawRunner()}
	hello := backendtest.RunnerHello(backendtest.RunnerVersion, token, nil)
	answers := make([]map[string]any, 2)
	backendtest.Concurrently(2, func(index int) {
		if err := sockets[index].TrySend(hello); err != nil {
			t.Error(err)
		}
	})
	backendtest.Concurrently(2, func(index int) {
		answer, err := sockets[index].TryNext()
		if err != nil {
			t.Error(err)
		}
		answers[index] = answer
	})
	var bound *backendtest.RawRunner
	var refused map[string]any
	switch {
	case answers[0] != nil && answers[0]["type"] == "hello_ok":
		bound, refused = sockets[0], answers[1]
	case answers[1] != nil && answers[1]["type"] == "hello_ok":
		bound, refused = sockets[1], answers[0]
	default:
		t.Fatalf("expected one welcome, got %v", answers)
	}
	if refused == nil || refused["type"] != "hello_error" || refused["code"] != "already_connected" {
		t.Fatalf("the other hello is answered with %v", refused)
	}
	if !b.Online(master, laptop.ID()) {
		t.Fatal("the device is not online")
	}
	// A second hello on the bound socket is no new registration: the backend
	// answers nothing, and the socket stays the device's. The backend handles a
	// runner's messages in order, so the refusal of a request sent after the
	// hello is the first answer that arrives.
	bound.Send(hello)
	bound.Send(map[string]any{"type": "pong", "jobs": 0})
	bound.Send(map[string]any{
		"type": "artifact_resolve", "id": "after-hello", "owner": map[string]any{"streamId": "none"},
		"sha256": strings.Repeat("0", 64), "target": string(backendtest.HostTarget(t)),
	})
	answer := bound.Next()
	if answer == nil || answer["type"] != "artifact_location" || answer["id"] != "after-hello" || answer["error"] == nil {
		t.Fatalf("expected the refusal of the later request first, got %v", answer)
	}
	if !b.Online(master, laptop.ID()) {
		t.Fatal("the repeated hello took the device offline")
	}
	bound.Close()
	b.UntilOnline(master, laptop.ID(), false)
	b.Stop()
}

// A native service's numbers come from its conversation's tab sequence, each
// once, and only for a conversation of the device's user that reaches the device
// (native-runtime.md § Conversation numbers).
//
// Cost: one backend and a real runner, about a second.
func TestARunnerReservesNumbersOnlyOfItsUsersConversationsThatReachIt(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	token := laptop.Token()
	here := newID()
	b.CreateConversation(master, here)
	b.SwitchTo(master, here, laptop, laptop.Home())
	// A conversation on the Cloud does not reach the laptop.
	elsewhere := newID()
	b.CreateConversation(master, elsewhere)
	laptop.Runner.Stop()
	b.UntilOnline(master, laptop.ID(), false)

	runner := b.ConnectRawRunner()
	runner.Send(backendtest.RunnerHello(backendtest.RunnerVersion, token, nil))
	if welcome := runner.Next(); welcome == nil || welcome["type"] != "hello_ok" {
		t.Fatalf("the hello is answered with %v", welcome)
	}
	reserve := func(conversation string, count int) (first float64, refused bool) {
		t.Helper()
		id := newID()
		runner.Send(map[string]any{
			"type": "numbers_reserve", "id": id, "conversationId": conversation, "sequence": "tab", "count": count,
		})
		for {
			message := runner.Next()
			if message == nil {
				t.Fatal("the backend closed the socket")
			}
			if message["type"] != "numbers_reserved" || message["id"] != id {
				continue
			}
			first, hasFirst := message["first"].(float64)
			_, hasError := message["error"]
			if hasFirst == hasError {
				t.Fatalf("an answer carries its first number or its error: %v", message)
			}
			return first, hasError
		}
	}
	if first, refused := reserve(here, 4); refused || first != 1 {
		t.Fatalf("the first four numbers start at %v (refused %v)", first, refused)
	}
	if first, refused := reserve(here, 1); refused || first != 5 {
		t.Fatalf("the next number is %v (refused %v)", first, refused)
	}
	if _, refused := reserve(elsewhere, 1); !refused {
		t.Fatal("a conversation that does not reach the device is given numbers")
	}
	if _, refused := reserve("no-such-conversation", 1); !refused {
		t.Fatal("a conversation that does not exist is given numbers")
	}
	// A refused request takes no number.
	if first, refused := reserve(here, 2); refused || first != 6 {
		t.Fatalf("after the refusals the next number is %v (refused %v)", first, refused)
	}
	runner.Close()
	b.Stop()
}

// Cost: one backend, about a second.
func TestAClaimWhoseRunnerWentAwayMakesNoDevice(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	runner := b.ConnectRawRunner()
	runner.Send(backendtest.RunnerHello(backendtest.RunnerVersion, "", nil))
	pending := runner.Next()
	if pending == nil || pending["type"] != "claim_pending" {
		t.Fatalf("the runner was given no code: %v", pending)
	}
	runner.Close()
	claimed := b.Post("/api/devices/claim", master, backendtest.Map{"code": pending["claimToken"]})
	wantRefusal(t, claimed, http.StatusNotFound, "invalid_code", "the claim of a runner that went away")
	if got := b.Devices(master); len(got) != 0 {
		t.Fatalf("the claim made devices %v", got)
	}
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestAPipeIsReachedOnlyWithADeviceToken(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	token := laptop.Token()
	pipe := "/api/pipes/0123456789abcdef"

	for _, method := range []string{http.MethodGet, http.MethodPut} {
		request := func(headers map[string]string) *backendtest.Answer {
			return b.Do(backendtest.Request{Method: method, Path: pipe, Raw: []byte("bytes"), Headers: headers})
		}
		anonymous := request(nil).Expect(http.StatusUnauthorized)
		if anonymous.Text() != "device token required" {
			t.Fatalf("%s without a token answers %q", method, anonymous.Text())
		}
		request(map[string]string{"Authorization": "Bearer not-a-token"}).Expect(http.StatusUnauthorized)
		// The browser's session is not a device's credential.
		request(map[string]string{"Cookie": master.Cookie}).Expect(http.StatusUnauthorized)
		request(map[string]string{"Authorization": "Bearer " + token}).Expect(http.StatusNotFound)
	}
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestAPairedDeviceIsBrowsedAndItsLogReadThroughDeviceAccess(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	home := laptop.Home()
	if err := os.WriteFile(filepath.Join(home, "hello.txt"), []byte("hi"), 0o644); err != nil {
		t.Fatal(err)
	}

	browsed := b.Get("/api/devices/"+laptop.ID()+"/fs", master).Expect(http.StatusOK)
	if browsed.Str("path") != home || browsed.Str("home") != home {
		t.Fatalf("the listing is %s", browsed.Body)
	}
	var hello map[string]any
	entries, _ := browsed.At("entries").([]any)
	for _, entry := range entries {
		if backendtest.At(entry, "name") == "hello.txt" {
			hello = entry.(map[string]any)
		}
	}
	if hello == nil || hello["isDirectory"] != false || hello["isSymbolicLink"] != false || hello["size"] != 2.0 {
		t.Fatalf("hello.txt is listed as %v", hello)
	}
	made := home + "/made/by/web"
	b.Post("/api/devices/"+laptop.ID()+"/fs", master, backendtest.Map{"path": made}).Expect(http.StatusCreated)
	if info, err := os.Stat(filepath.Join(home, "made/by/web")); err != nil || !info.IsDir() {
		t.Fatalf("the directory was not made: %v", err)
	}
	for _, refused := range []string{"?path=relative", "?path="} {
		wantRefusal(t, b.Get("/api/devices/"+laptop.ID()+"/fs"+refused, master), http.StatusBadRequest, "invalid_query", refused)
	}
	wantRefusal(t, b.Get("/api/devices/"+laptop.ID()+"/fs?path="+home+"/nothing", master), http.StatusNotFound, "fs_error", "a missing directory")

	// The log answers by cursor, limit and source; the runner writes a line to
	// its files a moment after the event, so the read is repeated until it holds
	// the line.
	path := "/api/devices/" + laptop.ID() + "/log"
	lines := func(answer *backendtest.Answer) []any {
		list, _ := answer.At("lines").([]any)
		return list
	}
	texts := func(list []any) []string {
		var texts []string
		for _, line := range list {
			texts = append(texts, backendtest.At(line, "text").(string))
		}
		return texts
	}
	tail := b.Get(path, master)
	for range 500 {
		if slices.Contains(texts(lines(tail)), "online") {
			break
		}
		time.Sleep(10 * time.Millisecond)
		tail = b.Get(path, master)
	}
	tailLines := lines(tail)
	if !slices.Contains(texts(tailLines), "online") || !slices.Contains(texts(tailLines), "waiting to be paired") {
		t.Fatalf("the log is %v", texts(tailLines))
	}
	for _, line := range tailLines {
		if backendtest.At(line, "source") != "runner" {
			t.Fatalf("a line of the log is %v", line)
		}
	}
	code := backendtest.PairingCode(t, laptop.Runner, 0)
	// The pairing code is for the console alone.
	for _, text := range texts(tailLines) {
		if strings.Contains(text, code) {
			t.Fatalf("the log holds the pairing code: %q", text)
		}
	}
	next := tail.At("next")
	after := b.Get(path+"?since="+jsonText(next), master)
	backendtest.AssertJSON(t, after.Value(), backendtest.Map{"lines": []any{}, "next": next})
	first := b.Get(path+"?since=0&limit=1", master)
	backendtest.AssertJSON(t, lines(first), tailLines[:1])
	second := b.Get(path+"?since="+jsonText(first.At("next"))+"&limit=1", master)
	backendtest.AssertJSON(t, lines(second), tailLines[1:2])
	other := b.Get(path+"?since=0&source=service%3Anone", master)
	backendtest.AssertJSON(t, other.Value(), backendtest.Map{"lines": []any{}, "next": next})
	for _, query := range []string{"limit=0", "limit=1001", "limit=many", "since=-1", "since=1.5", "source="} {
		wantRefusal(t, b.Get(path+"?"+query, master), http.StatusBadRequest, "invalid_query", query)
	}
	wantRefusal(t, b.Get("/api/devices/none/log", master), http.StatusNotFound, "device_not_found", "the log of a missing device")

	// Offline, the device answers 409 instead of waking anything.
	laptop.Runner.Stop()
	b.UntilOnline(master, laptop.ID(), false)
	wantRefusal(t, b.Get(path, master), http.StatusConflict, "device_offline", "the log of an offline device")
	wantRefusal(t, b.Get("/api/devices/"+laptop.ID()+"/fs", master), http.StatusConflict, "device_offline", "the listing of an offline device")
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestARunnerThatGoesAwayWhileItsTokenIsLookedUpIsLetGoAndNeverComesOnline(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	token := laptop.Token()
	laptop.Runner.Stop()
	b.UntilOnline(master, laptop.ID(), false)

	lookups := b.Control.Hold(controlproto.HoldHelloTokenLookup)
	runner := b.ConnectRawRunner()
	runner.Send(backendtest.RunnerHello(backendtest.RunnerVersion, token, nil))
	lookups.Wait(1)
	// The runner ends the connection while the lookup still waits.
	runner.Close()
	if message := runner.Next(); message != nil {
		t.Fatalf("the backend answers a runner that left: %v", message)
	}
	lookups.Release()
	// A runner that left while its token was looked up is never bound: the
	// device stays offline, and the backend stops cleanly.
	if b.Online(master, laptop.ID()) {
		t.Fatal("a runner that left came online")
	}
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestARunnerWhoseHelloMeetsTheShutdownIsNeverWelcomed(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	token := laptop.Token()
	laptop.Runner.Stop()
	b.UntilOnline(master, laptop.ID(), false)
	// A page's conversation socket, which the shard closes early in its own close,
	// before its runners' connections; the answer to a frame shows that the shard
	// serves it.
	b.CreateConversation(master, convFirst)
	page := b.Connect(master, convFirst)
	page.Send(backendtest.Frame{"type": "abort"})
	page.Frame()

	binds := b.Control.Hold(controlproto.HoldHelloBind)
	runner := b.ConnectRawRunner()
	runner.Send(backendtest.RunnerHello(backendtest.RunnerVersion, token, nil))
	binds.Wait(1)
	// What the control holds stays held through the shutdown.
	stopped := b.Terminate()
	if code := page.Closed(); code != 1001 {
		t.Fatalf("the page's socket closes with %d, not 1001 (going away)", code)
	}
	binds.Release()
	answer := runner.Next()
	// A welcomed runner would keep the shutdown waiting for it.
	runner.Close()
	if answer != nil {
		t.Fatalf("the backend welcomed a runner while it shut down: %v", answer)
	}
	if code := stopped(); code != 0 {
		t.Fatalf("the backend exited with %d", code)
	}
}

// Cost: one backend started twice and two real runners, about two seconds.
func TestAfterABackendRestartTheDevicesAreKeptAndTheirRunnersComeBack(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	laptop := b.Pair(master, "laptop")
	// Devices list oldest first, by the time they were paired.
	b.Control.AdvanceClock(time.Second)
	desktop := b.Pair(master, "desktop")
	b.Stop()

	// The session and the devices are records: both outlive the process. The
	// backend comes back at its address, where the runners reconnect.
	b = h.Start()
	for _, device := range []*backendtest.Paired{laptop, desktop} {
		b.UntilOnline(master, device.ID(), true)
	}
	var names []any
	for _, device := range b.Devices(master) {
		if device["lastSeenAt"] == nil {
			t.Fatalf("a device that came back was never seen: %v", device)
		}
		names = append(names, device["name"])
	}
	backendtest.AssertJSON(t, names, []any{"laptop", "desktop"})
	b.Stop()
}

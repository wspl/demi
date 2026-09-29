package backendtest_test

import (
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/controlproto"
	"github.com/wspl/demi/go/backendtest/scripted"
)

func renameConversation(b *backendtest.Backend, session *backendtest.Session, id, title string) {
	b.Patch("/api/conversations/"+id, session, backendtest.Map{"title": title}).Expect(http.StatusOK)
}

func setTheme(b *backendtest.Backend, session *backendtest.Session, theme string) {
	b.Patch("/api/settings/preferences", session, backendtest.Map{"appearance": backendtest.Map{"theme": theme}}).Expect(http.StatusOK)
}

// summaryOf is the summary a message carries, if it is one of the conversation.
func summaryOf(event backendtest.Frame, id string) map[string]any {
	if event["type"] != "conversation" || backendtest.At(event, "conversation.id") != id {
		return nil
	}
	summary, _ := event["conversation"].(map[string]any)
	return summary
}

// untilSummary waits until the channel brings the summary of the conversation
// that wanted accepts, and answers it.
func untilSummary(page *backendtest.SyncChannel, id string, wanted func(map[string]any) bool) map[string]any {
	received := page.Until(func(event backendtest.Frame) bool {
		summary := summaryOf(event, id)
		return summary != nil && wanted(summary)
	})
	return summaryOf(received[len(received)-1], id)
}

// themed reports whether the message carries preferences with the theme.
func themed(event backendtest.Frame, theme string) bool {
	return event["type"] == "preferences" && backendtest.At(event, "preferences.appearance.theme") == theme
}

// Cost: one backend, about a second.
func TestTheSnapshotIsTheUsersProductState(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	state := b.Sync(master).Snapshot()
	backendtest.AssertJSON(t, state, backendtest.Map{
		"user":         b.Get("/api/auth/me", master).At("user"),
		"mode":         "shared",
		"preferences":  backendtest.Map{"appearance": backendtest.Map{}, "shortcuts": backendtest.Map{}},
		"providers":    []any{},
		"workspaces":   []any{},
		"devices":      []any{},
		"exposes":      []any{},
		"exposeDomain": nil,
		// The URL runners connect to: the backend's own address, as the harness
		// configures it.
		"publicUrl":     b.URL + "/",
		"conversations": []any{},
		// A Cloud no work used yet is not made.
		"cloud": backendtest.Map{
			"device": nil, "state": "unallocated", "operation": nil, "error": nil, "volumes": nil,
			"limits": backendtest.Map{"systemBytes": 16 << 30, "homeBytes": 32 << 30},
		},
	})
	// The page's install command fetches the installer at that URL's origin,
	// which is this backend's; it has no runner releases to install.
	installer := b.Get("/install.sh", nil)
	if installer.Status != http.StatusServiceUnavailable {
		t.Fatalf("the installer answers %d", installer.Status)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestAChangeInOneSessionReachesTheOtherSessionsPageAsOneMessage(t *testing.T) {
	t.Parallel()
	b, laptop := backendtest.New(t).StartSetUp()
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.CreateConversation(laptop, convFirst)
	page := b.Sync(phone)
	state := page.Snapshot()
	if backendtest.At(state, "conversations.0.title") != "New conversation" || backendtest.At(state, "conversations.0.draftRevision") != 0.0 {
		t.Fatalf("the snapshot's conversation is %v", backendtest.At(state, "conversations.0"))
	}

	// Each change the laptop makes is one message on the phone's page: the
	// message after it is the next change's.
	renameConversation(b, laptop, convFirst, "Fix the login")
	renamed := page.Next()
	if got := summaryOf(renamed, convFirst)["title"]; got != "Fix the login" {
		t.Fatalf("the phone sees %v", renamed)
	}
	b.Put("/api/conversations/"+convFirst+"/draft", laptop, backendtest.Map{"base": 0, "text": "The login fails", "files": []any{}}).Expect(http.StatusOK)
	drafted := page.Next()
	if got := summaryOf(drafted, convFirst)["draftRevision"]; got != 1.0 {
		t.Fatalf("the phone sees %v", drafted)
	}
	b.Patch("/api/auth/me", laptop, backendtest.Map{"nickname": "Ana"}).Expect(http.StatusOK)
	if event := page.Next(); event["type"] != "user" || backendtest.At(event, "user.nickname") != "Ana" {
		t.Fatalf("the phone sees %v", event)
	}
	setTheme(b, laptop, "dark")
	if event := page.Next(); !themed(event, "dark") {
		t.Fatalf("the phone sees %v", event)
	}
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestATurnShowsOnEveryPageAsItRunsAndStopsAndReadingItOnOneClearsItOnAll(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, laptop := backendtest.New(t).StartSetUp()
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	provider := b.Anthropic(laptop, vendor, "")
	b.CreateConversation(laptop, convFirst)
	b.Choose(laptop, convFirst, provider, "claude-opus-4-8")
	page := b.Sync(phone)
	page.Snapshot()

	// The answer streams and stays open, so the turn runs until the laptop stops
	// it.
	frames := append([]scripted.Frame{scripted.MessageStart(scripted.Frame{"input_tokens": 12, "output_tokens": 0})}, scripted.TextBlock(0, []string{"Hello"})...)
	vendor.Respond(scripted.EventStream(scripted.Events(frames)).StayOpen())
	socket := b.Connect(laptop, convFirst)
	socket.Open()
	socket.Send(backendtest.SendMessage("m1", "Say hello"))
	untilSummary(page, convFirst, func(s map[string]any) bool { return s["status"] == "running" })
	socket.Send(backendtest.Frame{"type": "abort"})
	stopped := untilSummary(page, convFirst, func(s map[string]any) bool { return s["status"] == "stopped" })
	if stopped["unread"] != true || stopped["revision"].(float64) <= 0 {
		t.Fatalf("the stopped turn is %v", stopped)
	}

	b.Post("/api/conversations/"+convFirst+"/read", laptop, backendtest.Map{"revision": stopped["revision"]}).Expect(http.StatusNoContent)
	untilSummary(page, convFirst, func(s map[string]any) bool { return s["unread"] == false })
	b.Stop()
}

// A page tells a quiet socket from one that died without a close by the
// heartbeat each of its sockets sends once it has sent nothing else for the
// interval (web-application.md § Liveness and reconnection): the channel after
// its snapshot, and a conversation socket whose page never opened the
// conversation. Each waits for its heartbeat, 0.2 s here.
//
// Cost: one backend, about a second, of which 0.2 s is the interval.
func TestEachSocketOfAPageSendsAHeartbeatOnceItWasQuietForTheInterval(t *testing.T) {
	t.Parallel()
	const interval = 200 * time.Millisecond
	h := backendtest.New(t)
	h.Pages().HeartbeatMs = backendtest.Ptr(uint64(interval.Milliseconds()))
	b, master := h.StartSetUp()
	b.CreateConversation(master, convFirst)

	connected := time.Now()
	page := b.Sync(master)
	socket := b.Connect(master, convFirst)
	page.Snapshot()
	// Both sockets were quiet for the interval; the second's heartbeat waits in
	// its buffer while the first is read.
	event := page.Next()
	frame := socket.Frame()
	if event["type"] != "heartbeat" || frame["type"] != "heartbeat" {
		t.Fatalf("the sockets sent %v and %v", event, frame)
	}
	if time.Since(connected) < interval {
		t.Fatal("a heartbeat came before the interval")
	}
	b.Stop()
}

func pinConversation(b *backendtest.Backend, session *backendtest.Session, id string) {
	b.Patch("/api/conversations/"+id, session, backendtest.Map{"pinned": true}).Expect(http.StatusOK)
}

// Cost: one backend, about a second.
func TestAPageThatReconnectsCatchesUpOnWhatChangedWhileItWasAwayAndWhileItRead(t *testing.T) {
	t.Parallel()
	b, laptop := backendtest.New(t).StartSetUp()
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.CreateConversation(laptop, convFirst)
	away := b.Sync(phone)
	away.Snapshot()
	away.Close()
	renameConversation(b, laptop, convFirst, "Renamed while away")

	// The phone connects again; once its channel has read the state, and before
	// it sends it, the laptop pins the conversation.
	held := b.Control.Hold(controlproto.HoldSyncSnapshot)
	page := b.Sync(phone)
	held.Wait(1)
	pinConversation(b, laptop, convFirst)
	held.Release()
	state := page.Snapshot()
	if backendtest.At(state, "conversations.0.title") != "Renamed while away" || backendtest.At(state, "conversations.0.pinned") != false {
		t.Fatalf("the snapshot reads %v", backendtest.At(state, "conversations.0"))
	}
	// The pin, made after that read, comes after it: the summary, then the order.
	caughtUp := page.Until(func(event backendtest.Frame) bool { return event["type"] == "conversation_order" })
	if len(caughtUp) != 2 {
		t.Fatalf("the page caught up with %v", caughtUp)
	}
	if summary := summaryOf(caughtUp[0], convFirst); summary == nil || summary["pinned"] != true {
		t.Fatalf("the summary is %v", caughtUp[0])
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestAPageThatFallsBehindReceivesEachChangedPartOnceAsItIsWhenItReads(t *testing.T) {
	t.Parallel()
	b, laptop := backendtest.New(t).StartSetUp()
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.CreateConversation(laptop, convFirst)
	page := b.Sync(phone)
	page.Snapshot()

	// The phone's page takes nothing while the laptop renames the conversation
	// fifty times and changes a preference.
	held := b.Control.Hold(controlproto.HoldSyncChanges)
	for number := 1; number <= 50; number++ {
		renameConversation(b, laptop, convFirst, fmt.Sprintf("Title %d", number))
	}
	setTheme(b, laptop, "dark")
	held.Wait(1)
	held.Release()
	// It receives the conversation once, as it is now, then the preference.
	latest := summaryOf(page.Next(), convFirst)
	if latest == nil || latest["title"] != "Title 50" {
		t.Fatalf("the page receives %v", latest)
	}
	if event := page.Next(); !themed(event, "dark") {
		t.Fatalf("the page receives %v", event)
	}
	// It was not closed for falling behind.
	renameConversation(b, laptop, convFirst, "Caught up")
	if renamed := summaryOf(page.Next(), convFirst); renamed == nil || renamed["title"] != "Caught up" {
		t.Fatalf("the page receives %v", renamed)
	}
	b.Stop()
}

// upgradeRefusal is how the channel's route answers an upgrade before
// upgrading, from origin, with the session if any.
func upgradeRefusal(b *backendtest.Backend, session *backendtest.Session, origin string) (int, string) {
	headers := map[string]string{
		"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Version": "13",
		"Sec-WebSocket-Key": "MDEyMzQ1Njc4OWFiY2RlZg==",
	}
	if origin != "" {
		headers["Origin"] = origin
	}
	return b.Do(backendtest.Request{Path: "/api/sync", Session: session, Headers: headers}).Refusal()
}

// Cost: one backend, about a second.
func TestTheChannelOpensForASignedInPageOfTheProductAndEndsWithItsSessionOrTheBackend(t *testing.T) {
	t.Parallel()
	b, laptop := backendtest.New(t).StartSetUp()
	product := b.URL
	if status, code := upgradeRefusal(b, laptop, "https://a1b2c3.expose.localhost"); status != http.StatusForbidden || code != "forbidden_origin" {
		t.Fatalf("a foreign page is refused with %d %s", status, code)
	}
	if status, code := upgradeRefusal(b, nil, product); status != http.StatusUnauthorized || code != "unauthenticated" {
		t.Fatalf("a page without a session is refused with %d %s", status, code)
	}
	plain := b.Do(backendtest.Request{Path: "/api/sync", Session: laptop, Headers: map[string]string{"Origin": product}})
	wantRefusal(t, plain, http.StatusUpgradeRequired, "upgrade_required", "a request that is no upgrade")

	// Signing out closes the channels of that session, and no other's.
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	tablet := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	laptopPage := b.Sync(laptop)
	laptopPage.Snapshot()
	phonePage := b.Sync(phone)
	phonePage.Snapshot()
	b.Post("/api/auth/logout", phone, backendtest.Map{}).Expect(http.StatusNoContent)
	if code, reason := phonePage.Closed(); code != 4002 || reason != "session_ended" {
		t.Fatalf("the phone's channel closes with %d %s", code, reason)
	}

	// A channel never renews its session, not even as it opens: the tablet's page
	// opens 20 days after its sign-in, when a request would renew the session, and
	// closes as the session ends 30 days after the sign-in. The laptop's requests
	// renew the laptop's.
	b.Control.AdvanceClock(20 * day)
	setTheme(b, laptop, "dark")
	if event := laptopPage.Next(); !themed(event, "dark") {
		t.Fatalf("the laptop's page receives %v", event)
	}
	tabletPage := b.Sync(tablet)
	tabletPage.Snapshot()
	b.Control.AdvanceClock(11 * day)
	setTheme(b, laptop, "light")
	if event := laptopPage.Next(); !themed(event, "light") {
		t.Fatalf("the laptop's page receives %v", event)
	}
	if code, reason := tabletPage.Closed(); code != 4002 || reason != "session_ended" {
		t.Fatalf("the tablet's channel closes with %d %s", code, reason)
	}

	// A page sends nothing on its channel.
	laptopPage.SendText("hello")
	if code, reason := laptopPage.Closed(); code != 1008 || reason != "unexpected_message" {
		t.Fatalf("the laptop's channel closes with %d %s", code, reason)
	}

	// At shutdown the channels close first.
	last := b.Sync(laptop)
	last.Snapshot()
	stopped := b.Terminate()
	if code, reason := last.Closed(); code != 1001 || reason != "backend_closing" {
		t.Fatalf("the channel closes at shutdown with %d %s", code, reason)
	}
	if code := stopped(); code != 0 {
		t.Fatalf("the backend exited with %d", code)
	}
}

// untilProviders reads the channel up to the message that carries count entries,
// and answers them.
func untilProviders(page *backendtest.SyncChannel, count int) []any {
	received := page.Until(func(event backendtest.Frame) bool {
		providers, _ := event["providers"].([]any)
		return event["type"] == "providers" && len(providers) == count
	})
	providers, _ := received[len(received)-1]["providers"].([]any)
	return providers
}

// Cost: one backend and a scripted Codex, about a second.
func TestAnEntryReachesThePageOfEveryUserWhoInfersWithItAndOnlyAConfiguringUserSeesItsAccounts(t *testing.T) {
	t.Parallel()
	codex := scripted.StartCodex(t)
	b, master := backendtest.New(t, backendtest.WithCodex(codex)).StartSetUp()
	reader := b.CreateUser(master, "reader@example.test", "reader-pass-1", "user")
	masters := b.Sync(master)
	if providers, _ := masters.Snapshot()["providers"].([]any); len(providers) != 0 {
		t.Fatalf("a new instance has entries %v", providers)
	}
	readers := b.Sync(reader)
	readers.Snapshot()

	// The master adds entries of a shared instance, which every user infers with:
	// every user's page receives them.
	device := codexEntry(t, b, master, codex)
	b.Post("/api/providers", master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-state",
	}).Expect(http.StatusCreated)
	seen := untilProviders(masters, 2)
	var labels []any
	for _, entry := range seen {
		labels = append(labels, backendtest.At(entry, "label"))
	}
	backendtest.AssertJSON(t, labels, []any{"codex subscription", "Work"})
	if strings.Contains(jsonText(seen), "sk-state") {
		t.Fatal("the page receives an API key")
	}
	subscription := backendtest.At(seen[0], "details")
	if accounts, _ := backendtest.At(subscription, "accounts").([]any); len(accounts) != 1 || backendtest.At(subscription, "active") != backendtest.At(accounts[0], "id") {
		t.Fatalf("the subscription's details are %v", subscription)
	}
	if backendtest.At(seen[1], "details.auth.status") != "authenticated" || backendtest.At(seen[0], "id") != device["id"] {
		t.Fatalf("the entries are %v", seen)
	}
	// A user who only infers sees no accounts, plan or usage.
	seen = untilProviders(readers, 2)
	subscription = backendtest.At(seen[0], "details")
	if accounts, _ := backendtest.At(subscription, "accounts").([]any); len(accounts) != 0 || backendtest.At(subscription, "active") != nil || backendtest.At(subscription, "quota") != nil {
		t.Fatalf("a reader sees %v", subscription)
	}
	backendtest.AssertJSON(t, backendtest.At(subscription, "auth"), backendtest.Map{"status": "authenticated"})
	b.Stop()
}

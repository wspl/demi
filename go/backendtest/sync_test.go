package backendtest_test

import (
	"net/http"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
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

package backendtest_test

import (
	"encoding/json/v2"
	"net/http"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// generatedTitle is the title the scripted model writes.
const generatedTitle = "TS2307 after package split"

// titling is a scripted vendor that tells a title request from a turn by its
// system prompt: it records the title requests, and answers each once the test
// lets it, with the text the scenario chose.
type titling struct {
	mu     sync.Mutex
	asked  []any
	gate   *scripted.Gate
	answer string
}

func newTitling(t *testing.T, answer string) (*titling, *scripted.Vendor) {
	vendor := scripted.StartVendor(t)
	script := &titling{gate: scripted.NewGate(), answer: answer}
	vendor.Handle(func(request scripted.Request) *scripted.Response {
		var body any
		if err := json.Unmarshal(request.Body, &body); err != nil {
			return nil
		}
		system, _ := backendtest.At(body, "system.0.text").(string)
		if !strings.HasPrefix(system, "You are a title generator") {
			return scripted.Answer([]string{"Answer."}, 1, 1)
		}
		script.mu.Lock()
		script.asked = append(script.asked, body)
		script.mu.Unlock()
		return scripted.Answer([]string{script.answer}, 1, 1).After(script.gate)
	})
	return script, vendor
}

// count is how many title requests the vendor received.
func (s *titling) count() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.asked)
}

func (s *titling) request(index int) any {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.asked[index]
}

// input is the text of the user message a title request read.
func titleInput(request any) string {
	text, _ := backendtest.At(request, "messages.0.content.0.text").(string)
	return text
}

// titlingBackend starts a backend with titles on, whose master has an entry that
// configures the model m with an output limit of 8,000 tokens, and answers that
// entry.
func titlingBackend(t *testing.T, vendor *scripted.Vendor) (*backendtest.Backend, *backendtest.Session, string) {
	t.Helper()
	h := backendtest.New(t)
	h.Conversations().Titles = backendtest.Ptr(true)
	b, master := h.StartSetUp()
	provider := newEntry(b, master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "T", "apiKey": "k",
		"baseUrl": vendor.URL("/v1"), "models": []any{configuredModel(8_000)},
	})
	return b, master, provider
}

// Cost: one backend and a scripted vendor, about a second.
func TestATitleFollowsTheFirstMessageAndARenameOrAnArchiveWhileOneIsAskedWins(t *testing.T) {
	t.Parallel()
	script, vendor := newTitling(t, "\""+generatedTitle+"\"\n")
	b, master, provider := titlingBackend(t, vendor)
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "m")
	// The browser's record creation repeats the placeholder, which settles
	// nothing.
	b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{"title": "New conversation"}).Expect(http.StatusOK)
	socket := b.Connect(master, convFirst)
	socket.Open()

	// The first message is the title at once, and a request beside the first turn
	// writes a better one.
	message := "why does pnpm build fail with TS2307 after I moved auth into its own package"
	socket.Chat("m1", message)
	backendtest.Eventually(t, "the title request is asked", func() bool { return script.count() == 1 })
	asked := b.Summary(master, convFirst)
	if asked["title"] != message || asked["titleGenerating"] != true {
		t.Fatalf("the conversation while the title is asked is %v", asked)
	}
	script.gate.Permit(1)
	backendtest.Eventually(t, "the generated title is written", func() bool {
		return b.Summary(master, convFirst)["title"] == generatedTitle
	})
	titled := b.Summary(master, convFirst)
	if titled["titleGenerating"] != false || titled["titleCurrent"] != true {
		t.Fatalf("the titled conversation is %v", titled)
	}
	first := script.request(0)
	if got := titleInput(first); got != "1. "+message {
		t.Fatalf("the title request read %q", got)
	}
	// The request's own cap, although the configured model allows more
	// (models.md § Output limit).
	if backendtest.At(first, "max_tokens") != 1024.0 || backendtest.At(first, "tools") != nil || backendtest.At(first, "thinking") != nil {
		t.Fatalf("the title request is %s", jsonText(first))
	}
	// The request is metered like a turn.
	if got := backendtest.At(scenarioItem(t, usageTotals(b, master), 0), "requests"); got != 2.0 {
		t.Fatalf("the ledger counts %v requests", got)
	}

	// A later message titles nothing by itself, and the title is no longer
	// current; a rename that lands while Detect title's request is in flight
	// wins.
	socket.Chat("m2", "and the login test")
	if b.Summary(master, convFirst)["titleCurrent"] != false {
		t.Fatal("a later message left the title current")
	}
	path := "/api/conversations/" + convFirst + "/title"
	b.Post(path, master, backendtest.Map{}).Expect(http.StatusAccepted)
	backendtest.Eventually(t, "Detect title is asked", func() bool { return script.count() == 2 })
	if got := titleInput(script.request(1)); got != "1. "+message+"\n2. and the login test" {
		t.Fatalf("Detect title read %q", got)
	}
	b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{"title": "Kept"}).Expect(http.StatusOK)
	script.gate.Permit(1)
	backendtest.Eventually(t, "the request ends", func() bool { return b.Summary(master, convFirst)["titleGenerating"] == false })
	kept := b.Summary(master, convFirst)
	if kept["title"] != "Kept" || kept["titleCurrent"] != true {
		t.Fatalf("the renamed conversation is %v", kept)
	}

	// An archive ends a request in flight, which writes nothing.
	b.Post(path, master, backendtest.Map{})
	backendtest.Eventually(t, "the third request is asked", func() bool { return script.count() == 3 })
	b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	backendtest.Eventually(t, "the aborted request ends", func() bool {
		archived, _ := b.Get("/api/conversations?archived=true", master).At("conversations").([]any)
		return len(archived) > 0 && backendtest.At(archived[0], "titleGenerating") == false
	})
	if got := script.gate.Available(); got != 0 {
		t.Fatalf("%d permits are unspent", got)
	}
	wantRefusal(t, b.Post(path, master, backendtest.Map{}), http.StatusConflict, "conversation_archived", "a title of an archived conversation")

	// What there is nothing to title with is refused: a conversation without a
	// model, one without a message, and one whose provider is gone.
	b.CreateConversation(master, convSecond)
	second := "/api/conversations/" + convSecond + "/title"
	wantRefusal(t, b.Post(second, master, backendtest.Map{}), http.StatusConflict, "model_not_selected", "a title without a model")
	b.Choose(master, convSecond, provider, "m")
	wantRefusal(t, b.Post(second, master, backendtest.Map{}), http.StatusConflict, "no_messages", "a title without a message")
	b.Delete("/api/providers/"+provider, master).Expect(http.StatusNoContent)
	wantRefusal(t, b.Post(second, master, backendtest.Map{}), http.StatusNotFound, "provider_not_found", "a title without its provider")
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestAnAnswerWithoutATitleLeavesTheMessagesTitleAndDetectTitleAvailable(t *testing.T) {
	t.Parallel()
	script, vendor := newTitling(t, " \n\n")
	b, master, provider := titlingBackend(t, vendor)
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "m")
	socket := b.Connect(master, convFirst)
	socket.Open()
	socket.Chat("m1", "hello   there")
	backendtest.Eventually(t, "the title request is asked", func() bool { return script.count() == 1 })
	script.gate.Permit(1)
	backendtest.Eventually(t, "the request ends", func() bool { return b.Summary(master, convFirst)["titleGenerating"] == false })
	// The title in place stays, and asking again is the retry.
	left := b.Summary(master, convFirst)
	if left["title"] != "hello there" || left["titleCurrent"] != false {
		t.Fatalf("the conversation is %v", left)
	}
	b.Stop()
}

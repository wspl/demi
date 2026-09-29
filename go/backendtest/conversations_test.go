package backendtest_test

import (
	"bufio"
	"encoding/binary"
	"fmt"
	"net"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// The conversation ids the conversation scenarios create.
const (
	convFirst  = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b"
	convSecond = "7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a"
	convThird  = "5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d"
)

// jsonText is the JSON text of a value, as a request's body or a block shows it.
func jsonText(value any) string {
	return string(backendtest.Marshal(value))
}

// lastText is the text of the last text block.
func lastText(t *testing.T, blocks []any) string {
	t.Helper()
	for index := len(blocks) - 1; index >= 0; index-- {
		if backendtest.At(blocks[index], "type") == "text" {
			text, _ := backendtest.At(blocks[index], "text").(string)
			return text
		}
	}
	t.Fatal("the transcript holds text")
	return ""
}

// frameTypes is each frame's type.
func frameTypes(frames []backendtest.Frame) []string {
	types := make([]string, len(frames))
	for index, frame := range frames {
		types[index], _ = frame["type"].(string)
	}
	return types
}

func anyFrame(frames []backendtest.Frame, match func(backendtest.Frame) bool) bool {
	return slices.ContainsFunc(frames, match)
}

// configuredModel is a configured model of output tokens at most.
func configuredModel(output int) backendtest.Map {
	return backendtest.Map{
		"id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": output,
		"thinkingEfforts": []any{}, "acceptedExtensions": nil, "fastTier": nil,
	}
}

// newEntry creates an API-key entry of the master's and answers its id.
func newEntry(b *backendtest.Backend, master *backendtest.Session, body backendtest.Map) string {
	return b.Post("/api/providers", master, body).Expect(http.StatusCreated).Str("provider.id")
}

// usageTotals lists the caller's usage totals.
func usageTotals(b *backendtest.Backend, session *backendtest.Session) []any {
	totals, _ := b.Get("/api/usage", session).Expect(http.StatusOK).At("totals").([]any)
	return totals
}

// Cost: one backend, about a second.
func TestAConversationIsCreatedOnceUnderTheIDTheBrowserChoseAndListedForItsOwner(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")

	created := b.CreateConversation(master, convFirst).At("conversation")
	for path, want := range map[string]any{
		"id": convFirst, "title": "New conversation", "status": "idle", "revision": 0.0, "unread": false,
		"cwd": "/home/demi/sessions/" + convFirst,
	} {
		if got := backendtest.At(created, path); got != want {
			t.Fatalf("the new conversation's %s is %v, not %v", path, got, want)
		}
	}
	backendtest.AssertJSON(t, backendtest.At(created, "target"), backendtest.Map{"kind": "cloud"})

	// A retry, in any spelling, finds the one it created.
	for _, spelling := range []string{convFirst, strings.ToUpper(convFirst)} {
		again := b.Post("/api/conversations", master, backendtest.Map{"id": spelling}).Expect(http.StatusOK)
		backendtest.AssertJSON(t, again.At("conversation"), created)
	}
	second := b.CreateConversation(master, convSecond).At("conversation")
	if listed := conversationIDs(t, b, master, "/api/conversations"); !slices.Equal(listed, []string{convSecond, convFirst}) {
		t.Fatalf("the newest first: %v", listed)
	}
	if archived, _ := b.Get("/api/conversations?archived=true", master).At("conversations").([]any); len(archived) != 0 {
		t.Fatalf("the archived list holds %v", archived)
	}
	wantRefusal(t, b.Get("/api/conversations?archived=1", master), http.StatusBadRequest, "invalid_query", "a malformed query")
	state := b.Sync(master).Snapshot()
	backendtest.AssertJSON(t, state["conversations"], []any{second, created})
	if blocks := b.Transcript(master, convFirst); len(blocks) != 0 {
		t.Fatalf("a new conversation holds %v", blocks)
	}

	// Another user can neither take the id nor reach the conversation.
	wantRefusal(t, b.Post("/api/conversations", ana, backendtest.Map{"id": strings.ToUpper(convFirst)}), http.StatusConflict, "id_unavailable", "taking the id")
	for _, path := range []string{"/api/conversations/" + convFirst + "/transcript", "/api/conversations/not-a-uuid/transcript"} {
		wantRefusal(t, b.Get(path, ana), http.StatusNotFound, "conversation_not_found", path)
	}
	if listed := conversationIDs(t, b, ana, "/api/conversations"); len(listed) != 0 {
		t.Fatalf("ana lists %v", listed)
	}
	if _, status, _ := b.TryConnect(ana, convFirst); status != http.StatusNotFound {
		t.Fatalf("ana opens the socket: %d", status)
	}
	for _, body := range []backendtest.Map{{"id": "conversation-1"}, {"id": convFirst, "title": "x"}, {}} {
		wantRefusal(t, b.Post("/api/conversations", ana, body), http.StatusBadRequest, "invalid_body", jsonText(body))
	}
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestAMessageRunsOverTheSocketAndAReloadShowsWhatTheDatabaseHolds(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")

	socket := b.Connect(master, convFirst)
	handshake := socket.Open()
	if types := frameTypes(handshake); !slices.Equal(types, []string{"opened", "transcript_reset", "phase", "queue", "pending_steers"}) {
		t.Fatalf("the handshake is %v", types)
	}
	vendor.Respond(scripted.Answer([]string{"Hello", " there."}, 12, 3))
	turn := socket.Chat("m1", "Say hello")
	if !anyFrame(turn, func(f backendtest.Frame) bool { return f["type"] == "transcript_patch" }) {
		t.Fatalf("the turn has no patch: %v", turn)
	}

	sent := vendor.Requests()[0]
	if sent.Path != "/v1/messages" || sent.Header.Get("X-Api-Key") != "sk-ant-test" {
		t.Fatalf("the request is %s with key %q", sent.Path, sent.Header.Get("X-Api-Key"))
	}
	body := sent.JSON(t)
	system := jsonText(backendtest.At(body, "system"))
	contains(t, system, "You are a coding agent. Use shell session tools", "demi host")
	// Without the demi.builtin package the backend's own groups are offered, and
	// none of the package's.
	contains(t, system, "demi todo")
	if strings.Contains(system, "demi file") || strings.Contains(system, "demi browser") {
		t.Fatalf("the package's commands are offered: %s", system)
	}
	if backendtest.At(body, "model") != "claude-opus-4-8" {
		t.Fatalf("the model is %v", backendtest.At(body, "model"))
	}
	contains(t, jsonText(backendtest.At(body, "messages.0")), "Say hello")

	// Live equals cold once the page has seen the turn end: the live tree's
	// transcript is what the database holds, which the history route reads
	// without a session.
	summary := b.Summary(master, convFirst)
	live := socket.Live()
	if kinds := backendtest.BlockKinds(live); !slices.Equal(kinds, []string{"user", "text", "response"}) {
		t.Fatalf("the live transcript is %v", kinds)
	}
	if text := lastText(t, live); text != "Hello there." {
		t.Fatalf("the answer is %q", text)
	}
	cold := b.Get("/api/conversations/"+convFirst+"/transcript", master).Expect(http.StatusOK)
	backendtest.AssertJSON(t, cold.At("blocks"), live)
	if cold.At("failures") != nil || len(cold.At("subagents").([]any)) != 0 {
		t.Fatalf("the transcript has failures or subagents: %s", cold.Body)
	}

	if summary["status"] != "completed" || summary["unread"] != true || summary["revision"].(float64) <= 0 {
		t.Fatalf("the summary is %v", summary)
	}
	if backendtest.At(summary, "model.providerId") != provider || backendtest.At(summary, "model.modelId") != "claude-opus-4-8" {
		t.Fatalf("the model is %v", summary["model"])
	}
	revision := int(summary["revision"].(float64))
	wantRefusal(t, b.Post("/api/conversations/"+convFirst+"/read", master, backendtest.Map{"revision": revision + 1}),
		http.StatusConflict, "invalid_revision", "a read beyond the revision")
	b.Post("/api/conversations/"+convFirst+"/read", master, backendtest.Map{"revision": revision}).Expect(http.StatusNoContent)
	b.Post("/api/conversations/"+convFirst+"/read", master, backendtest.Map{"revision": 0}).Expect(http.StatusNoContent)
	after := b.Summary(master, convFirst)
	if after["readRevision"] != after["revision"] || after["unread"] != false {
		t.Fatalf("the summary after a read is %v", after)
	}

	// The request is metered: its usage is a ledger row.
	totals := usageTotals(b, master)
	if len(totals) != 1 ||
		backendtest.At(totals[0], "requests") != 1.0 || backendtest.At(totals[0], "inputTokens") != 12.0 ||
		backendtest.At(totals[0], "outputTokens") != 3.0 ||
		backendtest.At(totals[0], "providerId") != provider || backendtest.At(totals[0], "modelId") != "claude-opus-4-8" {
		t.Fatalf("the usage is %v", totals)
	}

	// A reload opens the same history, and a later message continues it.
	socket.Close()
	reloaded := b.Connect(master, convFirst)
	handshake = reloaded.Open()
	backendtest.AssertJSON(t, handshake[1]["blocks"], live)
	vendor.Respond(scripted.Answer([]string{"Again."}, 20, 2))
	reloaded.Chat("m2", "Once more")
	replayed := vendor.Requests()[1].JSON(t)
	if messages := backendtest.At(replayed, "messages").([]any); len(messages) != 3 {
		t.Fatalf("the replayed request has %d messages: %s", len(messages), jsonText(replayed))
	}
	if blocks := b.Transcript(master, convFirst); len(blocks) != 6 {
		t.Fatalf("the transcript holds %d blocks", len(blocks))
	}
	b.Stop()
}

// unmarked is value without the cache marks an Anthropic request carries, which
// the vendor does not count as content: what the vendor caches.
func unmarked(value any) any {
	switch value := value.(type) {
	case map[string]any:
		kept := map[string]any{}
		for key, field := range value {
			if key != "cache_control" {
				kept[key] = unmarked(field)
			}
		}
		return kept
	case []any:
		items := make([]any, len(value))
		for index, item := range value {
			items[index] = unmarked(item)
		}
		return items
	}
	return value
}

// Cost: one backend, a scripted vendor and four requests, about half a second.
func TestARequestTheVendorRefusesAsTooLargeCompactsAndGoesAgainFromTheSummary(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := newEntry(b, master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
		"baseUrl": vendor.URL("/v1"),
		"models": []any{backendtest.Map{
			"id": "model-a", "displayName": "A", "contextWindow": 200000, "outputLimit": 8000,
			"thinkingEfforts": []any{}, "acceptedExtensions": []any{}, "fastTier": nil,
		}},
	})
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "model-a")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"First answer."}, 12, 3))
	socket.Chat("m1", "First question")

	// The second request is refused as too large, as a compatible endpoint with a
	// smaller limit than the API's would.
	refusal := backendtest.Map{"type": "error", "error": backendtest.Map{"type": "request_too_large", "message": "Request exceeds the maximum size"}}
	vendor.Respond(scripted.Status(413).Header("content-type", "application/json").Chunk(backendtest.Marshal(refusal)))
	vendor.Respond(scripted.Answer([]string{"The user asked a first question."}, 10, 5))
	vendor.Respond(scripted.Answer([]string{"Second answer."}, 8, 2))
	turn := socket.Chat("m2", "Second question")

	// One pass: the summary request is the first request, which the vendor took,
	// with the instruction after it; then the refused request goes again, from
	// the summary.
	requests := vendor.Requests()
	if len(requests) != 4 {
		t.Fatalf("the vendor got %d requests", len(requests))
	}
	first, refused, summary, again := requests[0].JSON(t), requests[1].JSON(t), requests[2].JSON(t), requests[3].JSON(t)
	messages := func(body any) []any {
		list, _ := unmarked(backendtest.At(body, "messages")).([]any)
		return list
	}
	// Each block with its message's role: consecutive user items share a
	// message, and the vendor caches by block.
	blocks := func(body any) [][2]any {
		var list [][2]any
		for _, message := range messages(body) {
			role := backendtest.At(message, "role")
			content, _ := backendtest.At(message, "content").([]any)
			for _, block := range content {
				list = append(list, [2]any{role, block})
			}
		}
		return list
	}
	firstBlocks, summaryBlocks := blocks(first), blocks(summary)
	backendtest.AssertJSON(t, summaryBlocks[:len(firstBlocks)], firstBlocks)
	if len(summaryBlocks) != len(firstBlocks)+1 {
		t.Fatalf("the summary request adds %d blocks", len(summaryBlocks)-len(firstBlocks))
	}
	contains(t, jsonText(summaryBlocks[len(summaryBlocks)-1][1]), "Summarize the conversation above")
	backendtest.AssertJSON(t, unmarked(backendtest.At(summary, "system")), unmarked(backendtest.At(first, "system")))
	backendtest.AssertJSON(t, backendtest.At(summary, "tools"), backendtest.At(first, "tools"))
	if len(messages(refused)) != 3 {
		t.Fatalf("the refused request has %d messages", len(messages(refused)))
	}
	retried := messages(again)
	contains(t, jsonText(retried[0]), `Previous conversation summary:\nThe user asked a first question.`)
	contains(t, jsonText(retried[1]), "First answer.")
	contains(t, jsonText(retried[2]), "Second question")
	// The page saw the pass, and no retry: the refusal left nothing behind.
	if !anyFrame(turn, func(f backendtest.Frame) bool { return backendtest.IsPhase(f, "compacting") }) {
		t.Fatalf("the page saw no compaction: %v", frameTypes(turn))
	}
	if anyFrame(turn, func(f backendtest.Frame) bool { return f["type"] == "retry_scheduled" }) {
		t.Fatalf("the page saw a retry: %v", frameTypes(turn))
	}
	want := []string{"user", "compaction_boundary", "text", "response", "user", "compaction_marker", "text", "response"}
	if kinds := backendtest.BlockKinds(socket.Live()); !slices.Equal(kinds, want) {
		t.Fatalf("the live transcript is %v", kinds)
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a turn of two hundred deltas, about a
// second.
func TestAClientThatFallsBehindIsClosedAsLaggingAndAReopenAdoptsTheRunningTree(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	h.Conversations().OutboxFrames = backendtest.Ptr(uint64(16))
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	deltas := make([]string, 200)
	for index := range deltas {
		deltas[index] = strconv.Itoa(index) + " "
	}
	vendor.Respond(scripted.Answer(deltas, 5, 200))

	slow := b.Connect(master, convFirst)
	slow.Open()
	slow.Send(backendtest.SendMessage("m1", "Count"))
	if code := slow.Closed(); code != 4001 {
		t.Fatalf("the lagging client is closed with %d", code)
	}

	again := b.Connect(master, convFirst)
	again.Open()
	var blocks []any
	backendtest.Eventually(t, "the adopted turn ends", func() bool {
		blocks = again.Live()
		return slices.Contains(backendtest.BlockKinds(blocks), "response")
	})
	if text := lastText(t, blocks); !strings.HasSuffix(text, "199 ") {
		t.Fatalf("the turn did not run to its end: %q", text)
	}
	if got := len(vendor.Requests()); got != 1 {
		t.Fatalf("the reopen adopted the tree, which asked once, not %d times", got)
	}
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestTheFramesTheBackendRefusesNeverReachTheSession(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	socket := b.Connect(master, convFirst)

	errorCode := func(what string) string {
		t.Helper()
		frame := socket.Frame()
		if frame["type"] != "error" {
			t.Fatalf("%s is answered with %v, not an error", what, frame)
		}
		code, _ := frame["code"].(string)
		return code
	}
	// A frame outside its schema is answered, and the socket stays open.
	socket.SendText(`{"type":"send","messageId":"m1"}`)
	if code := errorCode("an invalid frame"); code != "invalid_frame" {
		t.Fatalf("an invalid frame is refused with %q", code)
	}
	// A conversation without a model opens no tree.
	socket.Send(backendtest.Frame{"type": "open"})
	if code := errorCode("an open without a model"); code != "model_not_selected" {
		t.Fatalf("an open without a model is refused with %q", code)
	}
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket.Open()

	// Archived, the conversation takes no frame but its close, and no socket.
	b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	socket.Send(backendtest.SendMessage("m1", "hi"))
	if code := errorCode("a message to an archived conversation"); code != "conversation_archived" {
		t.Fatalf("a message is refused with %q", code)
	}
	socket.Send(backendtest.Frame{
		"type": "edit_and_send",
		"request": backendtest.Map{
			"operationId": "edit-1", "targetBlockId": "user-1",
			"version": backendtest.Map{"epoch": "epoch", "revision": 1},
			"content": []any{backendtest.Map{"type": "text", "text": "edited"}},
		},
	})
	backendtest.AssertJSON(t, socket.Frame(), backendtest.Map{
		"type": "edit_result", "operationId": "edit-1",
		"outcome": backendtest.Map{"status": "rejected", "reason": "Restore the conversation before writing to it"},
	})
	if got := len(vendor.Requests()); got != 0 {
		t.Fatalf("the vendor got %d requests", got)
	}
	if _, status, _ := b.TryConnect(master, convFirst); status != http.StatusConflict {
		t.Fatalf("a new socket to an archived conversation: %d", status)
	}
	socket.Send(backendtest.Frame{"type": "close"})
	if frame := socket.Frame(); frame["type"] != "closed" {
		t.Fatalf("the close is answered with %v", frame)
	}

	// A message that is not JSON closes the socket.
	socket.SendText("not json")
	if code := socket.Closed(); code != 1007 {
		t.Fatalf("a message that is not JSON closes the socket with %d", code)
	}
	// A request from the product's page that is not an upgrade.
	product := map[string]string{"Origin": b.URL}
	path := "/api/conversations/" + convSecond + "/stream"
	wantRefusal(t, b.Do(backendtest.Request{Path: path, Session: master, Headers: product}), http.StatusNotFound, "conversation_not_found", "a plain request for a missing conversation")
	b.CreateConversation(master, convSecond)
	wantRefusal(t, b.Do(backendtest.Request{Path: path, Session: master, Headers: product}), http.StatusUpgradeRequired, "upgrade_required", "a plain request")
	b.Stop()
}

// A page on an expose shares the product's site, so the browser sends it the
// session cookie; the socket must still refuse it (backend.md § Authentication
// and ownership).
//
// Cost: one backend, about a second.
func TestTheConversationSocketOpensOnlyFromAPageOfTheProduct(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain("expose.localhost")).StartSetUp()
	b.CreateConversation(master, convFirst)
	// An expose's origin as the backend prints its URLs: the public URL's scheme
	// and port under the expose domain.
	expose := "http://a1b2c3d4e5.expose.localhost:" + strconv.Itoa(b.Port)
	for _, origin := range []string{"https://elsewhere.example", expose} {
		_, status, code := b.TryConnectFrom(master, convFirst, origin)
		if status != http.StatusForbidden || code != "forbidden_origin" {
			t.Fatalf("a socket from %s is %d %s", origin, status, code)
		}
	}
	// The product's own page opens it.
	b.Connect(master, convFirst).Close()
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestARequestOverTheRateLimitFailsWithoutReachingTheVendor(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	h.Conversations().RequestsPerMinute = backendtest.Ptr(uint64(1))
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()

	vendor.Respond(scripted.Answer([]string{"one"}, 1, 1))
	socket.Chat("m1", "first")
	turn := socket.Chat("m2", "second")

	if !anyFrame(turn, func(f backendtest.Frame) bool { return f["type"] == "error" && f["code"] == "rate_limited" }) {
		t.Fatalf("the turn is not rate limited: %v", turn)
	}
	if got := len(vendor.Requests()); got != 1 {
		t.Fatalf("the vendor got %d requests", got)
	}
	if got := backendtest.At(usageTotals(b, master)[0], "requests"); got != 1.0 {
		t.Fatalf("the ledger counts %v requests", got)
	}
	if status := b.Summary(master, convFirst)["status"]; status != "error" {
		t.Fatalf("the conversation is %v", status)
	}
	b.Stop()
}

// Cost: one backend started twice and a scripted vendor, about two seconds.
func TestAShutdownInTheMiddleOfATurnSavesItsInterruptionAndTheNextStartServesIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"first"}, 4, 1))
	socket.Chat("m1", "a first message")
	vendor.Respond(scripted.EventStream(": thinking\n\n").StayOpen())
	socket.Send(backendtest.SendMessage("m2", "take your time"))
	vendor.Received(t, 2)

	b.Stop()
	if code := socket.Closed(); code != 1001 {
		t.Fatalf("the socket closes with %d, not going away", code)
	}

	b = h.Start()
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	blocks := b.Transcript(master, convFirst)
	if kinds := backendtest.BlockKinds(blocks); !slices.Equal(kinds, []string{"user", "text", "response", "user", "error"}) {
		t.Fatalf("the transcript is %v", kinds)
	}
	if code := backendtest.At(blocks[len(blocks)-1], "code"); code != "interrupted" {
		t.Fatalf("the turn ends as %v", code)
	}
	if status := b.Summary(master, convFirst)["status"]; status != "interrupted" {
		t.Fatalf("the conversation is %v", status)
	}
	// The ledger carries its rows over the restart.
	if got := backendtest.At(usageTotals(b, master)[0], "requests"); got != 1.0 {
		t.Fatalf("the ledger counts %v requests", got)
	}

	// The next turn runs on the restored tree.
	socket = b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"done"}, 1, 1))
	socket.Chat("m3", "go on")
	if text := lastText(t, socket.Live()); text != "done" {
		t.Fatalf("the answer is %q", text)
	}
	b.Stop()
}

// stalledPage is a page's conversation socket that stops reading where the test
// says, with a receive buffer of a few kilobytes: a frame larger than the
// backend's send buffer then fills the socket's buffers, and the backend's send
// of it waits for a read that never comes.
type stalledPage struct {
	conn net.Conn
}

func connectStalledPage(t *testing.T, b *backendtest.Backend, session *backendtest.Session, conversation string) *stalledPage {
	t.Helper()
	dialer := net.Dialer{Control: func(_, _ string, raw syscall.RawConn) error {
		var failure error
		if err := raw.Control(func(fd uintptr) {
			failure = syscall.SetsockoptInt(int(fd), syscall.SOL_SOCKET, syscall.SO_RCVBUF, 4096)
		}); err != nil {
			return err
		}
		return failure
	}}
	host := strings.TrimPrefix(b.URL, "http://")
	conn, err := dialer.Dial("tcp", host)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	request := "GET /api/conversations/" + conversation + "/stream HTTP/1.1\r\nHost: " + host +
		"\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13" +
		"\r\nSec-WebSocket-Key: MDEyMzQ1Njc4OWFiY2RlZg==\r\nCookie: " + session.Cookie + "\r\nOrigin: " + b.URL + "\r\n\r\n"
	if _, err := conn.Write([]byte(request)); err != nil {
		t.Fatal(err)
	}
	// The answer's head, byte by byte: what follows it is the socket's.
	reader := bufio.NewReaderSize(conn, 1)
	head := ""
	for !strings.HasSuffix(head, "\r\n\r\n") {
		next, err := reader.ReadByte()
		if err != nil {
			t.Fatalf("the upgrade is not answered: %v", err)
		}
		head += string(next)
	}
	if !strings.HasPrefix(head, "HTTP/1.1 101") {
		t.Fatalf("the upgrade is answered with %s", head)
	}
	return &stalledPage{conn: conn}
}

// openUntilTheResetIsSent opens the conversation and reads the socket's bytes as
// far as the header of the frame after opened, the transcript's reset, whose
// length it answers: the backend is sending the reset, and the page reads no
// further.
func (p *stalledPage) openUntilTheResetIsSent(t *testing.T) uint64 {
	t.Helper()
	// A client's frame is masked: a final text frame of the open frame.
	payload := []byte(`{"type":"open"}`)
	mask := [4]byte{1, 2, 3, 4}
	frame := []byte{0x81, 0x80 | byte(len(payload))}
	frame = append(frame, mask[:]...)
	for index, value := range payload {
		frame = append(frame, value^mask[index%4])
	}
	if _, err := p.conn.Write(frame); err != nil {
		t.Fatal(err)
	}
	// Read beneath the WebSocket, which would read the whole reset: a final text
	// frame for opened, then the reset's frame header with its 64-bit length.
	opened := `{"type":"opened"}`
	head := make([]byte, 2+len(opened)+10)
	if err := p.conn.SetReadDeadline(time.Now().Add(backendtest.Patience)); err != nil {
		t.Fatal(err)
	}
	for read := 0; read < len(head); {
		n, err := p.conn.Read(head[read:])
		if err != nil {
			t.Fatalf("the backend sends the handshake: %v", err)
		}
		read += n
	}
	if head[0] != 0x81 || int(head[1]) != len(opened) || string(head[2:2+len(opened)]) != opened ||
		head[2+len(opened)] != 0x81 || head[3+len(opened)] != 127 {
		t.Fatalf("the handshake begins with %v", head)
	}
	return binary.BigEndian.Uint64(head[4+len(opened):])
}

// A page that stopped reading, its socket's buffers full, holds up shutdown no
// longer than the close frame's bound (backend.md § Startup and shutdown): the
// backend is in the middle of sending a transcript larger than any socket buffer
// when it shuts down. The 8 MiB message that makes the transcript so large costs
// most of the test's 2 s: a smaller one can fit the send buffer, whose limit is 4
// MiB on Linux and macOS.
//
// Cost: one backend and a scripted vendor, about two seconds.
func TestAPageThatStoppedReadingDoesNotHoldUpShutdown(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	h.Pages().CloseWaitMs = backendtest.Ptr(uint64(100))
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"noted"}, 1, 1))
	socket.Chat("m1", strings.Repeat("x", 8<<20))

	stalled := connectStalledPage(t, b, master, convFirst)
	if reset := stalled.openUntilTheResetIsSent(t); reset <= 8<<20 {
		t.Fatalf("the reset carries the whole transcript: %d bytes", reset)
	}
	// Stop fails the test when the backend does not stop within the patience.
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestThePageReceivesWhatTheProviderReadsFromAnErrorBlocksRecord(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Status(401).Header("retry-after", "120").
		Chunk([]byte(`{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}`)))
	turn := socket.Chat("m1", "hi")

	// The frame that brings the error block carries the provider's reading of its
	// record: the vendor's wait, counted from when it failed.
	var read map[string]any
	carrying := 0
	for _, frame := range turn {
		if failures, ok := frame["failures"].(map[string]any); ok && frame["type"] == "transcript_patch" {
			read = failures
			carrying++
		}
	}
	if read == nil {
		t.Fatalf("no frame carried failure facts: %v", frameTypes(turn))
	}
	transcript := b.Get("/api/conversations/"+convFirst+"/transcript", master).Expect(http.StatusOK)
	blocks := transcript.At("blocks").([]any)
	last := blocks[len(blocks)-1]
	if backendtest.At(last, "type") != "error" {
		t.Fatalf("the turn failed: %v", backendtest.BlockKinds(blocks))
	}
	// The retry-at time counts from the moment the request failed, on the
	// backend's clock, which starts at 08:00.
	facts := read[backendtest.At(last, "id").(string)]
	if backendtest.At(facts, "retryAt") != "2026-09-24T08:02:00.000Z" {
		t.Fatalf("the facts are %v", facts)
	}
	backendtest.AssertJSON(t, transcript.At("failures"), read)
	if carrying != 1 {
		t.Fatalf("%d frames carry facts, and only the one that brings the block should", carrying)
	}

	// With its entry gone, the record shows without facts.
	b.Delete("/api/providers/"+provider, master)
	if failures := b.Get("/api/conversations/"+convFirst+"/transcript", master).At("failures"); failures != nil {
		t.Fatalf("the record shows facts %v without its entry", failures)
	}
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second: the tool call boots
// the Cloud to run its shell job.
func TestADeepseekToolContinuationSendsTheReasoningBackToTheCompatibleEndpoint(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	catalog := backendtest.Map{"deepseek": backendtest.Map{
		"id": "deepseek", "name": "DeepSeek", "npm": "@ai-sdk/openai-compatible",
		"api": "https://api.deepseek.com", "models": backendtest.Map{},
	}}
	vendor.RespondAt("/api.json", scripted.Status(200).Header("etag", `"fixture"`).Chunk(backendtest.Marshal(catalog)))
	b, master := backendtest.New(t, backendtest.WithModelsDev(vendor.URL("/api.json"))).StartSetUp()
	provider := newEntry(b, master, backendtest.Map{
		"source": "vendor", "vendorId": "deepseek", "label": "DeepSeek", "apiKey": "fake-key",
		"baseUrl": vendor.URL("/v1"), "models": []any{configuredModel(8_000)},
	})
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "m")
	socket := b.Connect(master, convFirst)
	socket.Open()
	stream := func(delta backendtest.Map) *scripted.Response {
		return scripted.EventStream(fmt.Sprintf("data: %s\n\ndata: [DONE]\n\n", jsonText(backendtest.Map{"choices": []any{backendtest.Map{"delta": delta}}})))
	}
	vendor.Respond(stream(backendtest.Map{
		"reasoning_content": "Read the current directory.",
		"tool_calls": []any{backendtest.Map{"index": 0, "id": "call-1", "function": backendtest.Map{
			"name": "shell_exec", "arguments": jsonText(backendtest.Map{"script": "pwd", "timeoutMs": 1000}),
		}}},
	}))
	vendor.Respond(stream(backendtest.Map{"content": "done"}))

	socket.Chat("m1", "read the current directory")

	var requests []any
	for _, request := range vendor.Requests() {
		if request.Path == "/v1/chat/completions" {
			requests = append(requests, request.JSON(t))
		}
	}
	if len(requests) != 2 {
		t.Fatalf("the tool's result was not sent back: %d requests", len(requests))
	}
	messages, _ := backendtest.At(requests[1], "messages").([]any)
	var asked any
	for _, message := range messages {
		if backendtest.At(message, "tool_calls") != nil {
			asked = message
		}
	}
	if asked == nil {
		t.Fatalf("the continuation does not replay the tool call: %v", messages)
	}
	if backendtest.At(asked, "reasoning_content") != "Read the current directory." {
		t.Fatalf("the reasoning is %v", backendtest.At(asked, "reasoning_content"))
	}
	if text := lastText(t, socket.Live()); text != "done" {
		t.Fatalf("the answer is %q", text)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestEachFieldOfAPatchAppliesOnItsOwnAndAnArchivedConversationTakesOnlyItsRestore(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.CreateConversation(master, convSecond)
	path := "/api/conversations/" + convFirst
	listed := func() []string { return conversationIDs(t, b, master, "/api/conversations") }
	applied := func(field string) backendtest.Map { return backendtest.Map{"status": "applied", "field": field} }

	renamed := b.Patch(path, master, backendtest.Map{"title": "  Build failure  "}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, renamed.At("results"), []any{applied("title")})
	if renamed.Str("conversation.title") != "Build failure" {
		t.Fatalf("the title is %q", renamed.Str("conversation.title"))
	}
	// The target the conversation has already is no switch.
	chosen := b.Patch(path, master, backendtest.Map{
		"pinned": true, "model": backendtest.Map{"providerId": provider, "modelId": "claude-opus-4-8"},
		"target": backendtest.Map{"kind": "cloud"},
	}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, chosen.At("results"), []any{applied("pinned"), applied("model"), applied("target")})
	if chosen.At("conversation.pinned") != true || chosen.Str("conversation.model.providerId") != provider ||
		chosen.Str("conversation.model.modelId") != "claude-opus-4-8" {
		t.Fatalf("the conversation is %s", chosen.Body)
	}
	if got := listed(); !slices.Equal(got, []string{convFirst, convSecond}) {
		t.Fatalf("a pinned conversation leads: %v", got)
	}
	// A patch of one field that is refused answers that field's refusal.
	foreign := b.Patch(path, master, backendtest.Map{"model": backendtest.Map{"providerId": "someone-elses", "modelId": "m"}})
	wantRefusal(t, foreign, http.StatusNotFound, "provider_not_found", "a model of another entry")

	// The archive goes first, so the rename beside it is refused.
	archived := b.Patch(path, master, backendtest.Map{"title": "Renamed", "archived": true}).Expect(http.StatusMultiStatus)
	backendtest.AssertJSON(t, archived.At("results.0"), applied("archived"))
	failed := archived.At("results.1")
	if backendtest.At(failed, "status") != "failed" || backendtest.At(failed, "field") != "title" ||
		backendtest.At(failed, "code") != "conversation_archived" || backendtest.At(failed, "httpStatus") != 409.0 {
		t.Fatalf("the rename of an archived conversation is not refused: %v", failed)
	}
	if archived.At("conversation.archived") != true || archived.Str("conversation.title") != "Build failure" {
		t.Fatalf("the conversation is %s", archived.Body)
	}
	if got := listed(); !slices.Equal(got, []string{convSecond}) {
		t.Fatalf("the archived conversation is listed: %v", got)
	}
	backendtest.AssertJSON(t, b.Get("/api/conversations?archived=true", master).At("conversations"), []any{archived.At("conversation")})
	// Its history stays readable.
	if blocks := b.Transcript(master, convFirst); len(blocks) != 0 {
		t.Fatalf("the history is %v", blocks)
	}
	for _, body := range []backendtest.Map{{"pinned": false}, {"target": backendtest.Map{"kind": "cloud"}}} {
		wantRefusal(t, b.Patch(path, master, body), http.StatusConflict, "conversation_archived", jsonText(body))
	}
	b.Patch(path, master, backendtest.Map{"archived": false}).Expect(http.StatusOK)
	if got := listed(); !slices.Equal(got, []string{convFirst, convSecond}) {
		t.Fatalf("the restore keeps the place and the pin: %v", got)
	}

	// A body outside the patch's rules changes nothing, and neither does a
	// conversation the caller does not have.
	for _, body := range []backendtest.Map{
		{"title": "   "},
		{"title": strings.Repeat("x", 257)},
		{"name": "x"},
		{"model": backendtest.Map{"providerId": provider}},
		{"model": nil},
		{"archived": "yes"},
	} {
		wantRefusal(t, b.Patch(path, master, body), http.StatusBadRequest, "invalid_body", jsonText(body))
	}
	for _, foreignPath := range []string{path, "/api/conversations/not-a-uuid"} {
		wantRefusal(t, b.Patch(foreignPath, ana, backendtest.Map{"pinned": true}), http.StatusNotFound, "conversation_not_found", foreignPath)
	}
	if title := b.Get("/api/conversations", master).Str("conversations.0.title"); title != "Build failure" {
		t.Fatalf("the title is %q", title)
	}
	b.Stop()
}

// leveled is a configured model with the efforts low and high, whose Fast is the
// tier fast when there is one.
func leveled(id string, fast any) backendtest.Map {
	return backendtest.Map{
		"id": id, "displayName": strings.ToUpper(id), "contextWindow": 100000, "outputLimit": 4000,
		"thinkingEfforts": []any{"low", "high"}, "acceptedExtensions": nil, "fastTier": fast,
	}
}

// A conversation's model settings are one value that every page shows (web-api.md
// § Sidebar mutations, read state and page synchronization): a change names only
// its parts, so two pages that change different parts keep both; another page
// reads the change in its next snapshot; the conversation's next request uses it,
// whichever page sends it; and a tree opened after a change made while none was
// live opens with it.
//
// Cost: one backend and a scripted vendor, about a second.
func TestAChangeOfTheModelSettingsReachesEveryPageAndTheNextRequest(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := newEntry(b, master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
		"baseUrl": vendor.URL("/v1"), "models": []any{leveled("m", "priority"), leveled("n", nil)},
	})
	b.CreateConversation(master, convFirst)
	b.CreateConversation(master, convSecond)
	path := "/api/conversations/" + convFirst
	settings := func(model string, effort, tier any) backendtest.Map {
		return backendtest.Map{"providerId": provider, "modelId": model, "thinkingEffort": effort, "serviceTierId": tier}
	}
	chosen := b.Choose(master, convFirst, provider, "m")
	backendtest.AssertJSON(t, chosen.At("conversation.model"), settings("m", nil, nil))

	// Page A raises the effort, and page B, another sign-in that has not read
	// that, turns Fast on: the value ends with both, and page A reads it in its
	// next snapshot.
	other := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	socket := b.Connect(other, convFirst)
	socket.Open()
	raised := b.Patch(path, master, backendtest.Map{"thinkingEffort": "high"})
	backendtest.AssertJSON(t, raised.At("results"), []any{backendtest.Map{"status": "applied", "field": "thinking_effort"}})
	fast := b.Patch(path, other, backendtest.Map{"serviceTierId": "priority"})
	both := settings("m", "high", "priority")
	backendtest.AssertJSON(t, fast.At("conversation.model"), both)
	backendtest.AssertJSON(t, b.Summary(master, convFirst)["model"], both)

	// Page B's message runs with both.
	vendor.Respond(scripted.Answer([]string{"one"}, 1, 1))
	socket.Chat("m1", "first")
	sent := vendor.Requests()[0].JSON(t)
	if backendtest.At(sent, "model") != "m" || backendtest.At(sent, "output_config.effort") != "high" || backendtest.At(sent, "service_tier") != "priority" {
		t.Fatalf("the request is %s", jsonText(sent))
	}

	// Page A switches the model as its menu does, naming the effort the new model
	// lists; the new model has no Fast tier, so the tier goes. Page B's next
	// message runs with the new value.
	switched := b.Patch(path, master, backendtest.Map{
		"model": backendtest.Map{"providerId": provider, "modelId": "n"}, "thinkingEffort": "high",
	})
	backendtest.AssertJSON(t, switched.At("results"), []any{
		backendtest.Map{"status": "applied", "field": "model"}, backendtest.Map{"status": "applied", "field": "thinking_effort"},
	})
	backendtest.AssertJSON(t, switched.At("conversation.model"), settings("n", "high", nil))
	vendor.Respond(scripted.Answer([]string{"two"}, 1, 1))
	socket.Chat("m2", "second")
	sent = vendor.Requests()[1].JSON(t)
	if backendtest.At(sent, "model") != "n" || backendtest.At(sent, "output_config.effort") != "high" || backendtest.At(sent, "service_tier") != nil {
		t.Fatalf("the request is %s", jsonText(sent))
	}

	// A change while no tree is live is the record's, and the next open takes it.
	socket.Send(backendtest.Frame{"type": "close"})
	socket.UntilType("closed")
	b.Patch(path, master, backendtest.Map{"thinkingEffort": "low"}).Expect(http.StatusOK)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"three"}, 1, 1))
	socket.Chat("m3", "third")
	if effort := backendtest.At(vendor.Requests()[2].JSON(t), "output_config.effort"); effort != "low" {
		t.Fatalf("the third request carries the effort %v", effort)
	}

	// A part the model does not offer, a model the catalog does not list, and a
	// part for a conversation without a model are refused and change nothing.
	second := "/api/conversations/" + convSecond
	for _, refusal := range []struct {
		path   string
		body   backendtest.Map
		status int
		code   string
	}{
		{path, backendtest.Map{"serviceTierId": "priority"}, http.StatusConflict, "setting_unavailable"},
		{path, backendtest.Map{"thinkingEffort": "max"}, http.StatusConflict, "setting_unavailable"},
		{path, backendtest.Map{"model": backendtest.Map{"providerId": provider, "modelId": "x"}}, http.StatusNotFound, "model_not_found"},
		{second, backendtest.Map{"thinkingEffort": "low"}, http.StatusConflict, "model_not_selected"},
	} {
		wantRefusal(t, b.Patch(refusal.path, master, refusal.body), refusal.status, refusal.code, jsonText(refusal.body))
	}
	backendtest.AssertJSON(t, b.Summary(master, convFirst)["model"], settings("n", "low", nil))
	b.Stop()
}

// A model that cannot turn thinking off always thinks at an effort its settings
// name (models.md § A conversation's model settings): a choice that names none
// takes the model's default, else the first effort it lists, so what every page
// shows is what the request sends.
//
// Cost: one backend and a scripted vendor, about a second.
func TestAModelThatCannotTurnThinkingOffShowsTheEffortItsRequestsCarry(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	// The provider's own directory: its models level their thinking, cannot turn
	// it off, and name no default effort.
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	chosen := b.Choose(master, convFirst, provider, "claude-opus-4-8")
	if effort := chosen.At("conversation.model.thinkingEffort"); effort != "low" {
		t.Fatalf("the chosen effort is %v", effort)
	}
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"one"}, 1, 1))
	socket.Chat("m1", "first")
	sent := vendor.Requests()[0].JSON(t)
	if backendtest.At(sent, "thinking.type") != "adaptive" || backendtest.At(sent, "output_config.effort") != "low" {
		t.Fatalf("the request is %s", jsonText(sent))
	}

	// Asking for the model's default names that effort again.
	path := "/api/conversations/" + convFirst
	b.Patch(path, master, backendtest.Map{"thinkingEffort": "high"})
	reset := b.Patch(path, master, backendtest.Map{"thinkingEffort": nil})
	if effort := reset.At("conversation.model.thinkingEffort"); effort != "low" {
		t.Fatalf("the default effort is %v", effort)
	}
	b.Stop()
}

// Cost: one backend and a scripted vendor, about a second.
func TestAnArchiveRefusesRunningWorkAndHoldsTheOpenSocketUntilTheRestore(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.EventStream(": thinking\n\n").StayOpen())
	socket.Send(backendtest.SendMessage("m1", "take your time"))
	vendor.Received(t, 1)

	path := "/api/conversations/" + convFirst
	wantRefusal(t, b.Patch(path, master, backendtest.Map{"archived": true}), http.StatusConflict, "turn_in_flight", "an archive during a turn")

	// Stopped and saved, the turn no longer holds the conversation.
	socket.Stop()
	if status := b.Summary(master, convFirst)["status"]; status != "stopped" {
		t.Fatalf("the conversation is %v", status)
	}
	b.Patch(path, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	socket.Send(backendtest.SendMessage("m2", "still there?"))
	frame := socket.Frame()
	if frame["type"] != "error" || frame["code"] != "conversation_archived" {
		t.Fatalf("an archived conversation answers %v", frame)
	}
	if _, status, _ := b.TryConnect(master, convFirst); status != http.StatusConflict {
		t.Fatalf("a new socket to an archived conversation: %d", status)
	}

	// Restored, the socket that stayed open runs a turn again.
	b.Patch(path, master, backendtest.Map{"archived": false}).Expect(http.StatusOK)
	vendor.Respond(scripted.Answer([]string{"back"}, 1, 1))
	socket.Chat("m3", "and now?")
	if text := lastText(t, socket.Live()); text != "back" {
		t.Fatalf("the answer is %q", text)
	}
	if got := len(vendor.Requests()); got != 2 {
		t.Fatalf("the refused message reached the vendor: %d requests", got)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestABatchAnswersEachItemOnItsOwn(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")
	b.CreateConversation(master, convFirst)
	b.CreateConversation(master, convSecond)
	b.CreateConversation(ana, convThird)

	answered := b.Post("/api/conversations/batch", master, backendtest.Map{"items": []any{
		backendtest.Map{"id": convFirst, "patch": backendtest.Map{"pinned": true}},
		backendtest.Map{"id": convSecond, "patch": backendtest.Map{"archived": true, "title": "Old"}},
		backendtest.Map{"id": convThird, "patch": backendtest.Map{"pinned": true}},
	}}).Expect(http.StatusMultiStatus)
	results, _ := answered.At("results").([]any)
	if len(results) != 3 {
		t.Fatalf("one outcome per item: %v", results)
	}
	first := results[0]
	if backendtest.At(first, "status") != "updated" || backendtest.At(first, "id") != convFirst || backendtest.At(first, "conversation.pinned") != true {
		t.Fatalf("the first item is %v", first)
	}
	backendtest.AssertJSON(t, backendtest.At(first, "results"), []any{backendtest.Map{"status": "applied", "field": "pinned"}})
	second := results[1]
	if backendtest.At(second, "status") != "updated" || backendtest.At(second, "conversation.archived") != true ||
		backendtest.At(second, "results.0.status") != "applied" || backendtest.At(second, "results.0.field") != "archived" ||
		backendtest.At(second, "results.1.status") != "failed" || backendtest.At(second, "results.1.code") != "conversation_archived" {
		t.Fatalf("the second item is %v", second)
	}
	third := results[2]
	if backendtest.At(third, "status") != "refused" || backendtest.At(third, "id") != convThird || backendtest.At(third, "code") != "conversation_not_found" {
		t.Fatalf("another user's conversation is not refused: %v", third)
	}
	if b.Get("/api/conversations", ana).At("conversations.0.pinned") != false {
		t.Fatal("the refused item pinned another user's conversation")
	}

	tooMany := make([]any, 101)
	for index := range tooMany {
		tooMany[index] = backendtest.Map{"id": convFirst, "patch": backendtest.Map{}}
	}
	for _, body := range []backendtest.Map{
		{"items": []any{}}, {"items": tooMany}, {"items": []any{backendtest.Map{"id": convFirst}}},
	} {
		wantRefusal(t, b.Post("/api/conversations/batch", master, body), http.StatusBadRequest, "invalid_body", "a batch outside its rules")
	}
	b.Stop()
}

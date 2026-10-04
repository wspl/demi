package backend_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

const (
	conversationFirst  = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b"
	conversationSecond = "7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a"
)

// conversationHarness owns the assembled backend's temporary data and manager.
func conversationHarness(t *testing.T) (context.Context, *backendtest.Harness) {
	t.Helper()
	// The package deadline bounds the scenario, including builds and large
	// transfers under -race. Individual operation guards belong at their waits.
	ctx := t.Context()
	harness, _, err := backendtest.HostsHarness(ctx, t)
	wireMust(t, err)
	return ctx, harness
}

// conversationRequest drives a page's JSON request with its session cookie.
func conversationRequest(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	method, path, body string,
	status int,
) backendtest.Answer {
	t.Helper()
	var cookie *string
	if session != nil {
		cookie = &session.Cookie
	}
	var data json.RawMessage
	if body != "" {
		data = json.RawMessage(body)
	}
	a, err := backend.Send(ctx, method, path, cookie, data)
	wireMust(t, err)
	if a.Status != status {
		t.Fatalf("%s %s: HTTP %d, want %d: %s", method, path, a.Status, status, a.Body)
	}
	return a
}

// conversationDecode validates a page response through its generated decoder.
func conversationDecode[T any](t *testing.T, a backendtest.Answer, decode func([]byte) (T, error)) T {
	t.Helper()
	v, err := decode(a.Body)
	wireMust(t, err)
	return v
}

// conversationEqual reports differences in a scenario's observed contract values.
func conversationEqual[T any](t *testing.T, got, want T) {
	t.Helper()
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatalf("(-want +got):\n%s", diff)
	}
}

// conversationJSON encodes scenario input with the wire contract's escaping (no HTML escaping).
func conversationJSON(t *testing.T, v any) string {
	t.Helper()
	b, err := contract.EncodeJSON(v)
	wireMust(t, err)
	return string(b)
}

// conversationRefusal checks the HTTP boundary's typed refusal.
func conversationRefusal(t *testing.T, a backendtest.Answer, code webapi.ErrorCode) {
	t.Helper()
	e, err := a.ErrorBody()
	wireMust(t, err)
	conversationEqual(t, e.Code, code)
}

// conversationCreate creates the page-chosen conversation identifier.
func conversationCreate(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id string,
) webapi.ConversationSummary {
	t.Helper()
	a := conversationRequest(ctx, t, backend, session, "POST", "/api/conversations", `{"id":"`+id+`"}`, 201)
	return conversationDecode(t, a, webapi.DecodeConversationAnswer).Conversation
}

// TestConversationCreatedOnceAndListedOnlyForOwner checks that conversation creation is idempotent and
// scoped to its owner.
// A local backend accepts a caller-chosen ID once without running a model.
func TestConversationCreatedOnceAndListedOnlyForOwner(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "ana@example.test", "ana-pass-1", webapi.RoleUser))
	created := conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationEqual(t, string(created.ID), conversationFirst)
	conversationEqual(t, created.Title, "New conversation")
	conversationEqual(t, created.Status, webapi.ConversationStatusIdle)
	conversationEqual(t, created.Revision, uint64(0))
	conversationEqual(t, created.Unread, false)
	conversationEqual(t, created.Cwd, "/home/demi/sessions/"+conversationFirst)
	conversationEqual(t, conversationJSON(t, created.Target), `{"kind":"cloud"}`)
	for _, id := range []string{conversationFirst, strings.ToUpper(conversationFirst)} {
		a := conversationRequest(ctx, t, backend, &session, "POST", "/api/conversations", `{"id":"`+id+`"}`, 200)
		conversationEqual(t, conversationDecode(t, a, webapi.DecodeConversationAnswer).Conversation, created)
	}
	second := conversationCreate(ctx, t, backend, &session, conversationSecond)
	listed := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations", "", 200),
		webapi.DecodeConversations,
	).Conversations
	conversationEqual(t, len(listed), 2)
	conversationEqual(
		t,
		[]webapi.ConversationID{listed[0].ID, listed[1].ID},
		[]webapi.ConversationID{conversationSecond, conversationFirst},
	)
	archived := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations?archived=true", "", 200),
		webapi.DecodeConversations,
	)
	conversationEqual(t, len(archived.Conversations), 0)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations?archived=1", "", 400),
		webapi.ErrorCodeInvalidQuery,
	)
	page, err := backend.Sync(ctx, t, &session)
	wireMust(t, err)
	snapshot, err := page.Snapshot(ctx)
	wireMust(t, err)
	conversationEqual(t, snapshot.Conversations, []webapi.ConversationSummary{second, created})
	cold := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"GET",
			"/api/conversations/"+conversationFirst+"/transcript",
			"",
			200,
		),
		webapi.DecodeTranscript,
	)
	conversationEqual(t, len(cold.Blocks), 0)
	ana, err := backend.Login(ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&ana,
			"POST",
			"/api/conversations",
			`{"id":"`+strings.ToUpper(conversationFirst)+`"}`,
			409,
		),
		webapi.ErrorCodeIDUnavailable,
	)
	for _, id := range []string{conversationFirst, "not-a-uuid"} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &ana, "GET", "/api/conversations/"+id+"/transcript", "", 404),
			webapi.ErrorCodeConversationNotFound,
		)
	}
	own := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &ana, "GET", "/api/conversations", "", 200),
		webapi.DecodeConversations,
	)
	conversationEqual(t, len(own.Conversations), 0)
	_, refused, err := backend.ConversationFrom(ctx, t, &ana, conversationFirst, backend.URL)
	if err == nil {
		t.Fatal("foreign socket accepted")
	}
	conversationEqual(t, refused.Status, 404)
	for _, body := range []string{`{"id":"conversation-1"}`, `{"id":"` + conversationFirst + `","title":"x"}`, `{}`} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &ana, "POST", "/api/conversations", body, 400),
			webapi.ErrorCodeInvalidBody,
		)
	}
}

// conversationAnthropic registers a real adapter pointing only at a local vendor.
func conversationAnthropic(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	v *providertest.MockVendor,
) string {
	t.Helper()
	body := `{"source":"custom","providerType":"anthropic","label":"Work",` +
		`"apiKey":"sk-ant-test","baseUrl":` + conversationJSON(
		t,
		v.URL("/v1"),
	) + `}`
	a := conversationRequest(ctx, t, backend, session, "POST", "/api/providers", body, 201)
	return string(conversationDecode(t, a, webapi.DecodeProviderAnswer).Provider.ID)
}

// conversationChoose changes the persisted model just as a page's first send does.
func conversationChoose(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id, provider, model string,
) webapi.ConversationSummary {
	t.Helper()
	a := conversationRequest(
		ctx,
		t,
		backend,
		session,
		"PATCH",
		"/api/conversations/"+id,
		`{"model":{"providerId":"`+provider+`","modelId":"`+model+`"}}`,
		200,
	)
	return conversationDecode(t, a, webapi.DecodeConversationUpdate).Conversation
}

// conversationAnswer scripts Anthropic text deltas and usage, without a real model.
func conversationAnswer(t *testing.T, deltas []string, input, output int) providertest.MockResponse {
	t.Helper()
	frames := []string{
		fmt.Sprintf(
			`{"type":"message_start","message":{"id":"msg_1","type":"message",`+
				`"role":"assistant","model":"claude-opus-4-8","content":[],`+
				`"usage":{"input_tokens":%d,"output_tokens":0}}}`,
			input,
		),
		`{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}`,
	}
	for _, delta := range deltas {
		frames = append(
			frames,
			`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":`+conversationJSON(
				t,
				delta,
			)+`}}`,
		)
	}
	frames = append(
		frames,
		`{"type":"content_block_stop","index":0}`,
		fmt.Sprintf(`{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":%d}}`, output),
		`{"type":"message_stop"}`,
	)
	var stream strings.Builder
	for _, frame := range frames {
		fields, err := contract.Object([]byte(frame))
		wireMust(t, err)
		kind, err := contract.Decode[string](fields["type"])
		wireMust(t, err)
		fmt.Fprintf(&stream, "event: %s\ndata: %s\n\n", kind, frame)
	}
	return providertest.EventStream(stream.String())
}

// conversationTranscript reads the stored transcript through the cold HTTP route.
func conversationTranscript(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id string,
) webapi.Transcript {
	t.Helper()
	return conversationDecode(
		t,
		conversationRequest(ctx, t, backend, session, "GET", "/api/conversations/"+id+"/transcript", "", 200),
		webapi.DecodeTranscript,
	)
}

// conversationSummary reads the sidebar's current view of this conversation.
func conversationSummary(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id string,
) webapi.ConversationSummary {
	t.Helper()
	all := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, session, "GET", "/api/conversations", "", 200),
		webapi.DecodeConversations,
	)
	for _, c := range all.Conversations {
		if string(c.ID) == id {
			return c
		}
	}
	t.Fatal("conversation missing from list")
	return webapi.ConversationSummary{}
}

// conversationKinds reports wire variant names without reproducing their mapping.
func conversationKinds[T any](t *testing.T, values []T) []string {
	t.Helper()
	kinds := make([]string, len(values))
	for i, v := range values {
		fields, err := contract.Object([]byte(conversationJSON(t, v)))
		wireMust(t, err)
		kinds[i], err = contract.Decode[string](fields["type"])
		wireMust(t, err)
	}
	return kinds
}

// conversationLastText finds the last assistant text visible in stored history.
func conversationLastText(t *testing.T, blocks []core.Block) string {
	t.Helper()
	for i := len(blocks) - 1; i >= 0; i-- {
		if b, ok := blocks[i].(*core.TextBlock); ok {
			return b.Text
		}
	}
	t.Fatal("transcript has no text")
	return ""
}

// TestMessageSocketReloadMatchesStoredTranscript checks that reloaded socket history matches the
// durable transcript.
// One local vendor and a real backend, with two turns and a socket reload.
func TestMessageSocketReloadMatchesStoredTranscript(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	socket, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	handshake, err := socket.Open(ctx)
	wireMust(t, err)
	conversationEqual(
		t,
		conversationKinds(t, handshake),
		[]string{"opened", "transcript_reset", "phase", "queue", "pending_steers"},
	)
	vendor.Respond(conversationAnswer(t, []string{"Hello", " there."}, 12, 3))
	turn, err := socket.Chat(ctx, "m1", "Say hello")
	wireMust(t, err)
	if !slices.Contains(conversationKinds(t, turn), "transcript_patch") {
		t.Fatal("turn sent no patches")
	}
	sent := vendor.Requests()[0]
	conversationEqual(t, sent.URI, "/v1/messages")
	conversationEqual(t, sent.Header("x-api-key"), "sk-ant-test")
	fields, err := contract.Object(sent.Body)
	wireMust(t, err)
	for _, text := range []string{"You are a coding agent. Use shell session tools", "demi host", "demi todo"} {
		if !strings.Contains(string(fields["system"]), text) {
			t.Fatalf("system lacks %q: %s", text, fields["system"])
		}
	}
	if strings.Contains(string(fields["system"]), "demi file") ||
		strings.Contains(string(fields["system"]), "demi browser") {
		t.Fatal("unpublished package offered")
	}
	conversationEqual(t, string(fields["model"]), `"claude-opus-4-8"`)
	messages, err := contract.Decode[[]json.RawMessage](fields["messages"])
	wireMust(t, err)
	if !strings.Contains(string(messages[0]), "Say hello") {
		t.Fatal("request lost user text")
	}
	summary := conversationSummary(ctx, t, backend, &session, conversationFirst)
	live, err := socket.Live(ctx)
	wireMust(t, err)
	conversationEqual(t, conversationKinds(t, live), []string{"user", "text", "response"})
	conversationEqual(t, conversationLastText(t, live), "Hello there.")
	cold := conversationTranscript(ctx, t, backend, &session, conversationFirst)
	conversationEqual(t, cold.Blocks, live)
	if cold.Failures != nil || len(cold.Subagents) != 0 {
		t.Fatal("unexpected failures or subagents")
	}
	conversationEqual(t, summary.Status, webapi.ConversationStatusCompleted)
	conversationEqual(t, summary.Unread, true)
	if summary.Revision == 0 {
		t.Fatal("turn did not advance revision")
	}
	conversationEqual(t, string(summary.Model.ProviderID), provider)
	conversationEqual(t, string(summary.Model.ModelID), "claude-opus-4-8")
	path := "/api/conversations/" + conversationFirst + "/read"
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			path,
			fmt.Sprintf(`{"revision":%d}`, summary.Revision+1),
			409,
		),
		webapi.ErrorCodeInvalidRevision,
	)
	conversationRequest(ctx, t, backend, &session, "POST", path, fmt.Sprintf(`{"revision":%d}`, summary.Revision), 204)
	conversationRequest(ctx, t, backend, &session, "POST", path, `{"revision":0}`, 204)
	summary = conversationSummary(ctx, t, backend, &session, conversationFirst)
	conversationEqual(t, summary.ReadRevision, summary.Revision)
	conversationEqual(t, summary.Unread, false)
	totals := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	).Totals
	conversationEqual(t, len(totals), 1)
	conversationEqual(t, totals[0].Requests, uint64(1))
	conversationEqual(t, totals[0].InputTokens, uint64(12))
	conversationEqual(t, totals[0].OutputTokens, uint64(3))
	conversationEqual(t, string(totals[0].ProviderID), provider)
	conversationEqual(t, string(totals[0].ModelID), "claude-opus-4-8")
	wireMust(t, socket.Close(ctx))
	reloaded, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	handshake, err = reloaded.Open(ctx)
	wireMust(t, err)
	reset, ok := handshake[1].(*framewire.TranscriptResetFrame)
	if !ok {
		t.Fatalf("unexpected handshake: %v", handshake)
	}
	conversationEqual(t, reset.Blocks, live)
	vendor.Respond(conversationAnswer(t, []string{"Again."}, 20, 2))
	_, err = reloaded.Chat(ctx, "m2", "Once more")
	wireMust(t, err)
	fields, err = contract.Object(vendor.Requests()[1].Body)
	wireMust(t, err)
	messages, err = contract.Decode[[]json.RawMessage](fields["messages"])
	wireMust(t, err)
	conversationEqual(t, len(messages), 3)
	conversationEqual(t, len(conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks), 6)
}

// conversationOpen attaches a page and consumes the standard handshake.
func conversationOpen(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id string,
) *backendtest.ConversationSocket {
	t.Helper()
	socket, err := backend.Conversation(ctx, t, session, id)
	wireMust(t, err)
	_, err = socket.Open(ctx)
	wireMust(t, err)
	return socket
}

// TestConversationSocketRequiresProductOrigin checks that conversation sockets enforce the product
// origin.
func TestConversationSocketRequiresProductOrigin(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	domain, err := expose.ParseDomain("expose.localhost")
	wireMust(t, err)
	harness.Config.ExposeDomain = &domain
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	for _, origin := range []string{
		"https://elsewhere.example",
		fmt.Sprintf("http://a1b2c3d4e5.expose.localhost:%d", backend.Address().Port()),
	} {
		conversationUpgradeRefusal(
			ctx,
			t,
			backend,
			&session,
			"/api/conversations/"+conversationFirst+"/stream",
			origin,
			403,
			webapi.ErrorCodeForbiddenOrigin,
		)
	}
	_, err = backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
}

// TestRefusedFramesNeverReachSession checks that invalid frames are refused before reaching the
// session.
func TestRefusedFramesNeverReachSession(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	socket, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	expectError := func(code string) {
		f, err := socket.Next(ctx)
		wireMust(t, err)
		e, ok := f.(*framewire.ErrorFrame)
		if !ok {
			t.Fatalf("expected error %s, got %T", code, f)
		}
		conversationEqual(t, e.Code, &code)
	}
	wireMust(t, socket.Text(ctx, `{"type":"send","messageId":"m1"}`))
	expectError("invalid_frame")
	wireMust(t, socket.Send(ctx, &framewire.OpenFrame{}))
	expectError("model_not_selected")
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	_, err = socket.Open(ctx)
	wireMust(t, err)
	db, err := harness.ControlDatabase(ctx, t)
	wireMust(t, err)
	_, err = db.ExecContext(ctx, "UPDATE conversations SET archived=1 WHERE id=?", conversationFirst)
	wireMust(t, err)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m1", "hi")))
	expectError("conversation_archived")
	edit := &framewire.EditAndSendFrame{
		Request: framewire.EditRequest{
			OperationID:   "edit-1",
			TargetBlockID: "user-1",
			Version:       framewire.TranscriptVersion{Epoch: "epoch", Revision: 1},
			Content:       backendtest.ConversationText("m1", "edited").Content,
		},
	}
	wireMust(t, socket.Send(ctx, edit))
	frame, err := socket.Next(ctx)
	wireMust(t, err)
	conversationEqual[framewire.ServerFrame](
		t,
		frame,
		&framewire.EditResultFrame{
			OperationID: "edit-1",
			Outcome:     &framewire.RejectedEdit{Reason: "Restore the conversation before writing to it"},
		},
	)
	conversationEqual(t, len(vendor.Requests()), 0)
	_, refused, err := backend.ConversationFrom(ctx, t, &session, conversationFirst, backend.URL)
	if err == nil {
		t.Fatal("archived stream accepted")
	}
	conversationEqual(t, refused.Status, 409)
	wireMust(t, socket.Send(ctx, &framewire.CloseFrame{}))
	frame, err = socket.Next(ctx)
	wireMust(t, err)
	conversationEqual[framewire.ServerFrame](t, frame, &framewire.ClosedFrame{})
	wireMust(t, socket.Text(ctx, "not json"))
	code, _, err := socket.Closed(ctx)
	wireMust(t, err)
	conversationEqual(t, int(code), 1007)
	path := "/api/conversations/" + conversationSecond + "/stream"
	for _, status := range []int{404, 426} {
		if status == 426 {
			conversationCreate(ctx, t, backend, &session, conversationSecond)
		}
		a, err := backend.ReadWith(ctx, path, &session, http.Header{"Origin": {backend.URL}})
		wireMust(t, err)
		conversationEqual(t, a.Status, status)
		want := webapi.ErrorCodeConversationNotFound
		if status == 426 {
			want = webapi.ErrorCodeUpgradeRequired
		}
		conversationRefusal(t, a, want)
	}
}

// TestRateLimitedRequestNeverReachesVendor checks that rate limiting prevents a vendor request.
func TestRateLimitedRequestNeverReachesVendor(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Conversations.RequestsPerMinute = 1
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"one"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "first")
	wireMust(t, err)
	frames, err := socket.Chat(ctx, "m2", "second")
	wireMust(t, err)
	limited := false
	for _, f := range frames {
		if e, ok := f.(*framewire.ErrorFrame); ok && e.Code != nil && *e.Code == "rate_limited" {
			limited = true
		}
	}
	if !limited {
		t.Fatal("missing rate_limited frame")
	}
	conversationEqual(t, len(vendor.Requests()), 1)
	totals := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	conversationEqual(t, totals.Totals[0].Requests, uint64(1))
	conversationEqual(
		t,
		conversationSummary(ctx, t, backend, &session, conversationFirst).Status,
		webapi.ConversationStatusError,
	)
}

// TestShutdownPersistsInterruptedTurnForRestart checks that shutdown preserves interrupted turns for
// restart.
func TestShutdownPersistsInterruptedTurnForRestart(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"first"}, 4, 1))
	_, err = socket.Chat(ctx, "m1", "a first message")
	wireMust(t, err)
	pending := providertest.EventStream(": thinking\n\n")
	pending.Ending = providertest.Open
	vendor.Respond(pending)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m2", "take your time")))
	vendor.Received(ctx, 2)
	wireMust(t, backend.Close(ctx))
	code, _, err := socket.Closed(ctx)
	wireMust(t, err)
	conversationEqual(t, int(code), 1001)
	backend, err = harness.Start(ctx, t)
	wireMust(t, err)
	session, err = backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	blocks := conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks
	conversationEqual(t, conversationKinds(t, blocks), []string{"user", "text", "response", "user", "error"})
	interrupted, ok := blocks[len(blocks)-1].(*core.ErrorBlock)
	if !ok {
		t.Fatal("no interruption")
	}
	conversationEqual(t, *interrupted.Code, "interrupted")
	conversationEqual(
		t,
		conversationSummary(ctx, t, backend, &session, conversationFirst).Status,
		webapi.ConversationStatusInterrupted,
	)
	totals := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	conversationEqual(t, totals.Totals[0].Requests, uint64(1))
	socket = conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"done"}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "go on")
	wireMust(t, err)
	blocks, err = socket.Live(ctx)
	wireMust(t, err)
	conversationEqual(t, conversationLastText(t, blocks), "done")
}

// conversationToolUse scripts one Anthropic tool call using the real adapter.
func conversationToolUse(t *testing.T, id, name, input string) providertest.MockResponse {
	t.Helper()
	frames := []string{
		`{"type":"message_start","message":{"id":"msg_1","type":"message",` +
			`"role":"assistant","model":"claude-opus-4-8","content":[],` +
			`"usage":{"input_tokens":1,"output_tokens":0}}}`,
		`{"type":"content_block_start","index":0,` +
			`"content_block":{"type":"tool_use","id":"` + id + `","name":"` + name + `","input":{}}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":` + conversationJSON(
			t,
			input,
		) + `}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}`,
		`{"type":"message_stop"}`,
	}
	var stream strings.Builder
	for _, f := range frames {
		fields, err := contract.Object([]byte(f))
		wireMust(t, err)
		kind, err := contract.Decode[string](fields["type"])
		wireMust(t, err)
		fmt.Fprintf(&stream, "event: %s\ndata: %s\n\n", kind, f)
	}
	return providertest.EventStream(stream.String())
}

// conversationShell scripts a model request to run a shell job.
func conversationShell(t *testing.T, id, script string, timeout int) providertest.MockResponse {
	t.Helper()
	return conversationToolUse(
		t,
		id,
		"shell_exec",
		fmt.Sprintf(
			`{"description":%s,"script":%s,"timeoutMs":%d}`,
			conversationJSON(t, id),
			conversationJSON(t, script),
			timeout,
		),
	)
}

// conversationOnDevice pairs a laptop and points the conversation's target at a
// work directory on it, writing the control database directly.
func conversationOnDevice(
	ctx context.Context,
	t *testing.T,
	harness *backendtest.Harness,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id string,
) (*backendtest.Paired, string) {
	t.Helper()
	paired, err := backend.Pair(ctx, t, session, "laptop")
	wireMust(t, err)
	root := filepath.Join(paired.Runner.Home(), "work")
	wireMust(t, os.MkdirAll(root, 0o755))
	db, err := harness.ControlDatabase(ctx, t)
	wireMust(t, err)
	_, err = db.ExecContext(
		ctx,
		"UPDATE conversations SET target_kind='device',target_device_id=?,target_path=? WHERE id=?",
		string(paired.ID()),
		root,
		id,
	)
	wireMust(t, err)
	return paired, root
}

// conversationToolResult reads the text that the vendor saw for a tool call.
func conversationToolResult(t *testing.T, request providertest.RecordedRequest, id string) string {
	t.Helper()
	fields, err := contract.Object(request.Body)
	wireMust(t, err)
	messages, err := contract.Decode[[]json.RawMessage](fields["messages"])
	wireMust(t, err)
	for _, message := range messages {
		m, err := contract.Object(message)
		wireMust(t, err)
		content, err := contract.Decode[[]json.RawMessage](m["content"])
		if err != nil {
			continue
		} // A plain user text is not a tool-result message.
		for _, block := range content {
			b, err := contract.Object(block)
			wireMust(t, err)
			if string(b["type"]) != `"tool_result"` || string(b["tool_use_id"]) != conversationJSON(t, id) {
				continue
			}
			parts, err := contract.Decode[[]json.RawMessage](b["content"])
			wireMust(t, err)
			first, err := contract.Object(parts[0])
			wireMust(t, err)
			text, err := contract.Decode[string](first["text"])
			wireMust(t, err)
			return text
		}
	}
	t.Fatalf("no result for %s in %s", id, request.Body)
	return ""
}

// TestMandatoryThinkingEffortMatchesRequest checks that the configured thinking effort reaches the
// vendor.
func TestMandatoryThinkingEffortMatchesRequest(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	chosen := conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	conversationEqual(t, *chosen.Model.ThinkingEffort, "low")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"one"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "first")
	wireMust(t, err)
	fields, err := contract.Object(vendor.Requests()[0].Body)
	wireMust(t, err)
	thinking, err := contract.Object(fields["thinking"])
	wireMust(t, err)
	output, err := contract.Object(fields["output_config"])
	wireMust(t, err)
	conversationEqual(t, string(thinking["type"]), `"adaptive"`)
	conversationEqual(t, string(output["effort"]), `"low"`)
	path := "/api/conversations/" + conversationFirst
	conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"thinkingEffort":"high"}`, 200)
	reset := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"thinkingEffort":null}`, 200),
		webapi.DecodeConversationUpdate,
	)
	conversationEqual(t, *reset.Conversation.Model.ThinkingEffort, "low")
}

// TestArchiveRefusesRunningWorkAndSocketResumesAfterRestore checks that archiving refuses running work
// and restoration permits reopening.
func TestArchiveRefusesRunningWorkAndSocketResumesAfterRestore(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	pending := providertest.EventStream(": thinking\n\n")
	pending.Ending = providertest.Open
	vendor.Respond(pending)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m1", "take your time")))
	vendor.Received(ctx, 1)
	path := "/api/conversations/" + conversationFirst
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"archived":true}`, 409),
		webapi.ErrorCodeTurnInFlight,
	)
	wireMust(t, socket.Stop(ctx))
	conversationEqual(
		t,
		conversationSummary(ctx, t, backend, &session, conversationFirst).Status,
		webapi.ConversationStatusStopped,
	)
	conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"archived":true}`, 200)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m2", "still there?")))
	frame, err := socket.Next(ctx)
	wireMust(t, err)
	e, ok := frame.(*framewire.ErrorFrame)
	if !ok {
		t.Fatalf("expected archived error, got %T", frame)
	}
	conversationEqual(t, *e.Code, "conversation_archived")
	_, refused, err := backend.ConversationFrom(ctx, t, &session, conversationFirst, backend.URL)
	if err == nil {
		t.Fatal("archived socket admitted")
	}
	conversationEqual(t, refused.Status, 409)
	conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"archived":false}`, 200)
	vendor.Respond(conversationAnswer(t, []string{"back"}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "and now?")
	wireMust(t, err)
	live, err := socket.Live(ctx)
	wireMust(t, err)
	conversationEqual(t, conversationLastText(t, live), "back")
	conversationEqual(t, len(vendor.Requests()), 2)
}

// TestOverdueWakeupRunsAfterBackendRestart checks that an overdue wakeup runs after restart.
func TestOverdueWakeupRunsAfterBackendRestart(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationToolUse(t, "toolu_wait", "yield", `{"durationMs":600000}`))
	_, err = socket.Chat(ctx, "m1", "start the build")
	wireMust(t, err)
	asked := len(vendor.Requests())
	wireMust(t, socket.Close(ctx))
	wireMust(t, backend.Close(ctx))
	wireMust(t, harness.Clock.Advance(11*time.Minute))
	vendor.Respond(conversationAnswer(t, []string{"the build passed"}, 1, 1))
	backend, err = harness.Start(ctx, t)
	wireMust(t, err)
	vendor.Received(ctx, asked+1)
	fields, err := contract.Object(vendor.Requests()[asked].Body)
	wireMust(t, err)
	if !strings.Contains(string(fields["messages"]), "Scheduled yield wakeup fired") {
		t.Fatalf("no wakeup input: %s", fields["messages"])
	}
	session, err = backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	page, state := conversationPage(ctx, t, backend, &session)
	if state.Conversations[0].Status != webapi.ConversationStatusCompleted {
		_, err = page.Until(ctx, func(e webapi.SyncEvent) bool {
			c, ok := e.(*webapi.SyncEventConversation)
			return ok && c.Conversation.Status == webapi.ConversationStatusCompleted
		})
		wireMust(t, err)
	}
	kinds := conversationKinds(t, conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks)
	if len(kinds) < 3 {
		t.Fatalf("short transcript: %v", kinds)
	}
	conversationEqual(t, kinds[len(kinds)-3:], []string{"wakeup", "text", "response"})
}

// TestProviderFailureFactsReachPageAndDisappearWithEntry checks that provider failures reach the page
// and disappear after deletion.
func TestProviderFailureFactsReachPageAndDisappearWithEntry(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, entry, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(
		providertest.MockResponse{
			Status:  401,
			Headers: http.Header{"Retry-After": {"120"}},
			Chunks: [][]byte{
				[]byte(`{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}`),
			},
		},
	)
	frames, err := socket.Chat(ctx, "m1", "hi")
	wireMust(t, err)
	var facts *framewire.Failures
	count := 0
	for _, f := range frames {
		if p, ok := f.(*framewire.TranscriptPatchFrame); ok && p.Failures != nil {
			facts = p.Failures
			count++
		}
	}
	if facts == nil {
		t.Fatal("no frame carried failure facts")
	}
	cold := conversationTranscript(ctx, t, backend, &session, conversationFirst)
	last, ok := cold.Blocks[len(cold.Blocks)-1].(*core.ErrorBlock)
	if !ok {
		t.Fatal("turn did not fail")
	}
	retry := core.Timestamp("2026-09-24T08:02:00.000Z")
	conversationEqual(t, (*facts)[last.ID()].RetryAt, &retry)
	conversationEqual(t, cold.Failures, facts)
	conversationEqual(t, count, 1)
	conversationRequest(ctx, t, backend, &session, "DELETE", "/api/providers/"+entry, "", 204)
	conversationEqual(
		t,
		conversationTranscript(ctx, t, backend, &session, conversationFirst).Failures,
		(*framewire.Failures)(nil),
	)
}

// TestOversizedVendorRequestCompactsAndReplaysSummary checks that oversized requests compact history
// and replay its summary.
func TestOversizedVendorRequestCompactsAndReplaysSummary(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"custom","providerType":"anthropic","label":"Work",` +
		`"apiKey":"sk-ant-test","baseUrl":` + conversationJSON(
		t,
		vendor.URL("/v1"),
	) + `,"models":[{"id":"model-a","displayName":"A","contextWindow":200000,` +
		`"outputLimit":8000,"thinkingEfforts":[],"acceptedExtensions":[],` +
		`"fastTier":null}]}`
	entry := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/providers", body, 201),
		webapi.DecodeProviderAnswer,
	).Provider.ID
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, string(entry), "model-a")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"First answer."}, 12, 3))
	_, err = socket.Chat(ctx, "m1", "First question")
	wireMust(t, err)
	vendor.Respond(
		providertest.MockResponse{
			Status:  413,
			Headers: http.Header{"Content-Type": {"application/json"}},
			Chunks: [][]byte{
				[]byte(
					`{"type":"error","error":{"type":"request_too_large","message":"Request exceeds the maximum size"}}`,
				),
			},
		},
	)
	vendor.Respond(conversationAnswer(t, []string{"The user asked a first question."}, 10, 5))
	vendor.Respond(conversationAnswer(t, []string{"Second answer."}, 8, 2))
	turn, err := socket.Chat(ctx, "m2", "Second question")
	wireMust(t, err)
	requests := vendor.Requests()
	conversationEqual(t, len(requests), 4)
	firstFixed, firstBlocks := wireCached(t, "anthropic", string(requests[0].Body), false)
	summaryFixed, summaryBlocks := wireCached(t, "anthropic", string(requests[2].Body), false)
	conversationEqual(t, summaryBlocks[:len(firstBlocks)], firstBlocks)
	conversationEqual(t, len(summaryBlocks), len(firstBlocks)+1)
	if !strings.Contains(fmt.Sprint(summaryBlocks[len(summaryBlocks)-1]), "Summarize the conversation above") {
		t.Fatal("missing summary instruction")
	}
	conversationEqual(t, summaryFixed["system"], firstFixed["system"])
	firstFields, err := contract.Object(requests[0].Body)
	wireMust(t, err)
	summaryFields, err := contract.Object(requests[2].Body)
	wireMust(t, err)
	firstTools, err := contract.Decode[any](firstFields["tools"])
	wireMust(t, err)
	summaryTools, err := contract.Decode[any](summaryFields["tools"])
	wireMust(t, err)
	conversationEqual(t, summaryTools, firstTools)
	refused, err := contract.Object(requests[1].Body)
	wireMust(t, err)
	messages, err := contract.Decode[[]json.RawMessage](refused["messages"])
	wireMust(t, err)
	conversationEqual(t, len(messages), 3)
	again, err := contract.Object(requests[3].Body)
	wireMust(t, err)
	messages, err = contract.Decode[[]json.RawMessage](again["messages"])
	wireMust(t, err)
	for i, text := range []string{
		`Previous conversation summary:\nThe user asked a first question.`,
		"First answer.",
		"Second question",
	} {
		if !strings.Contains(string(messages[i]), text) {
			t.Fatalf("replay %d lacks %s: %s", i, text, messages[i])
		}
	}
	compacted := false
	for _, f := range turn {
		if p, ok := f.(*framewire.PhaseFrame); ok && p.Phase == core.SessionPhaseCompacting {
			compacted = true
		}
		if _, ok := f.(*framewire.RetryScheduledFrame); ok {
			t.Fatal("size refusal scheduled retry")
		}
	}
	if !compacted {
		t.Fatal("page never saw compaction")
	}
	live, err := socket.Live(ctx)
	wireMust(t, err)
	conversationEqual(
		t,
		conversationKinds(t, live),
		[]string{"user", "compaction_boundary", "text", "response", "user", "compaction_marker", "text", "response"},
	)
}

// TestLaggingSocketClosesAndReopenAdoptsRunningTree checks that a lagging socket closes and reopening
// adopts the running tree.
// About 27 s under -race: an 8-MiB reset exceeds the TCP send buffer, so lag
// and adoption of a running turn are observed without relying on scheduling.
func TestLaggingSocketClosesAndReopenAdoptsRunningTree(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Conversations.OutboxFrames = 16
	advance := make(chan struct{})
	vendor := providertest.NewScriptedRuntime(t, func(ctx context.Context, _ provider.InferenceRequest) provider.Run {
		return func(yield func(provider.Event) bool) {
			if !yield(providertest.Thinking(strings.Repeat("x", 8<<20))) {
				return
			}
			for i := range 200 {
				select {
				case <-advance:
				case <-ctx.Done():
					return
				}
				if !yield(providertest.Text(fmt.Sprintf("%d ", i))) {
					return
				}
			}
			yield(providertest.Response(5, 200))
		}
	})
	harness.Config.Families.Register(
		"lag",
		conversationFamily{build: func(providers.FamilyArgs) provider.Runtime { return vendor }},
	)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"custom","providerType":"lag","label":"Lag","apiKey":"fixture","models":[` + conversationConfigured(
		4000,
	) + `]}`
	entry := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/providers", body, 201),
		webapi.DecodeProviderAnswer,
	).Provider.ID
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, string(entry), "m")
	progress, _ := conversationPage(ctx, t, backend, &session)
	reading := conversationOpen(ctx, t, backend, &session, conversationFirst)
	wireMust(t, reading.Send(ctx, backendtest.ConversationText("m1", "Count")))
	_, err = reading.Until(ctx, func(f framewire.ServerFrame) bool {
		patch, ok := f.(*framewire.TranscriptPatchFrame)
		if !ok {
			return false
		}
		for _, p := range patch.Patches {
			if add, ok := p.(*framewire.AddPatch); ok {
				thinking, ok := add.Value.(*core.ThinkingBlock)
				if ok && len(thinking.Text) == 8<<20 {
					return true
				}
			}
		}
		return false
	})
	wireMust(t, err)
	// The shard drains pages concurrently, so this page must really stop
	// reading inside a large reset to overflow.
	slow, err := backend.StallConversationReset(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	// Each acknowledged delta lets the healthy page drain while the slow
	// page keeps its reset unread. This does not depend on goroutine scheduling.
	for i := range 200 {
		if i == 32 {
			code, reason, err := slow.Closed(ctx)
			wireMust(t, err)
			conversationEqual(t, int(code), 4001)
			conversationEqual(t, reason, "lagged")
			again := conversationOpen(ctx, t, backend, &session, conversationFirst)
			conversationEqual(
				t,
				conversationSummary(ctx, t, backend, &session, conversationFirst).Status,
				webapi.ConversationStatusRunning,
			)
			wireMust(t, reading.Close(ctx))
			reading = again
		}
		select {
		case advance <- struct{}{}:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
		_, err = reading.Until(ctx, func(f framewire.ServerFrame) bool {
			patch, ok := f.(*framewire.TranscriptPatchFrame)
			if !ok {
				return false
			}
			want := fmt.Sprintf("%d ", i)
			for _, p := range patch.Patches {
				if delta, ok := p.(*framewire.AppendTextPatch); ok && delta.Delta == want {
					return true
				}
				if add, ok := p.(*framewire.AddPatch); ok {
					text, ok := add.Value.(*core.TextBlock)
					if ok && text.Text == want {
						return true
					}
				}
			}
			return false
		})
		wireMust(t, err)
	}
	titleUntil(
		ctx,
		t,
		progress,
		func(c webapi.ConversationSummary) bool { return c.Status == webapi.ConversationStatusCompleted },
	)
	live, err := reading.Live(ctx)
	wireMust(t, err)
	if !strings.HasSuffix(conversationLastText(t, live), "199 ") {
		t.Fatal("adopted turn did not finish")
	}
	conversationEqual(t, len(vendor.Requests()), 1)
}

// TestPatchFieldsApplyIndependentlyAndArchiveAllowsOnlyRestore checks that patch fields apply
// independently and archived conversations only admit restoration.
func TestPatchFieldsApplyIndependentlyAndArchiveAllowsOnlyRestore(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "ana@example.test", "ana-pass-1", webapi.RoleUser))
	entry := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationCreate(ctx, t, backend, &session, conversationSecond)
	path := "/api/conversations/" + conversationFirst
	patch := func(body string, status int) webapi.ConversationUpdate {
		return conversationDecode(
			t,
			conversationRequest(ctx, t, backend, &session, "PATCH", path, body, status),
			webapi.DecodeConversationUpdate,
		)
	}
	listed := func() []webapi.ConversationID {
		all := conversationDecode(
			t,
			conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations", "", 200),
			webapi.DecodeConversations,
		)
		ids := make([]webapi.ConversationID, len(all.Conversations))
		for i, c := range all.Conversations {
			ids[i] = c.ID
		}
		return ids
	}
	applied := func(fields ...webapi.PatchField) []webapi.FieldResult {
		results := make([]webapi.FieldResult, len(fields))
		for i, f := range fields {
			results[i] = &webapi.FieldResultApplied{Field: f}
		}
		return results
	}
	update := patch(`{"title":"  Build failure  "}`, 200)
	conversationEqual(t, update.Results, applied(webapi.PatchFieldTitle))
	conversationEqual(t, update.Conversation.Title, "Build failure")
	update = patch(
		`{"pinned":true,"model":{"providerId":"`+entry+`","modelId":"claude-opus-4-8"},"target":{"kind":"cloud"}}`,
		200,
	)
	conversationEqual(
		t,
		update.Results,
		applied(webapi.PatchFieldPinned, webapi.PatchFieldModel, webapi.PatchFieldTarget),
	)
	conversationEqual(t, update.Conversation.Pinned, true)
	conversationEqual(t, string(update.Conversation.Model.ProviderID), entry)
	conversationEqual(t, string(update.Conversation.Model.ModelID), "claude-opus-4-8")
	conversationEqual(t, listed(), []webapi.ConversationID{conversationFirst, conversationSecond})
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"PATCH",
			path,
			`{"model":{"providerId":"someone-elses","modelId":"m"}}`,
			404,
		),
		webapi.ErrorCodeProviderNotFound,
	)
	update = patch(`{"title":"Renamed","archived":true}`, 207)
	conversationEqual[webapi.FieldResult](
		t,
		update.Results[0],
		&webapi.FieldResultApplied{Field: webapi.PatchFieldArchived},
	)
	failed, ok := update.Results[1].(*webapi.FieldResultFailed)
	if !ok {
		t.Fatal("archived rename accepted")
	}
	conversationEqual(t, failed.Field, webapi.PatchFieldTitle)
	conversationEqual(t, failed.Code, webapi.ErrorCodeConversationArchived)
	conversationEqual(t, failed.HTTPStatus, uint16(409))
	conversationEqual(t, update.Conversation.Archived, true)
	conversationEqual(t, update.Conversation.Title, "Build failure")
	conversationEqual(t, listed(), []webapi.ConversationID{conversationSecond})
	archived := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations?archived=true", "", 200),
		webapi.DecodeConversations,
	)
	conversationEqual(t, archived.Conversations, []webapi.ConversationSummary{update.Conversation})
	conversationEqual(t, len(conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks), 0)
	for _, body := range []string{`{"pinned":false}`, `{"target":{"kind":"cloud"}}`} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "PATCH", path, body, 409),
			webapi.ErrorCodeConversationArchived,
		)
	}
	patch(`{"archived":false}`, 200)
	conversationEqual(t, listed(), []webapi.ConversationID{conversationFirst, conversationSecond})
	for _, body := range []string{
		`{"title":"   "}`,
		`{"title":"` + strings.Repeat("x", 257) + `"}`,
		`{"name":"x"}`,
		`{"model":{"providerId":"` + entry + `"}}`,
		`{"model":null}`,
		`{"archived":"yes"}`,
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "PATCH", path, body, 400),
			webapi.ErrorCodeInvalidBody,
		)
	}
	ana, err := backend.Login(ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	for _, p := range []string{path, "/api/conversations/not-a-uuid"} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &ana, "PATCH", p, `{"pinned":true}`, 404),
			webapi.ErrorCodeConversationNotFound,
		)
	}
	conversationEqual(t, conversationSummary(ctx, t, backend, &session, conversationFirst).Title, "Build failure")
}

// TestBatchAnswersEachConversationIndependently checks that batch results preserve each conversation's
// independent outcome.
func TestBatchAnswersEachConversationIndependently(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "ana@example.test", "ana-pass-1", webapi.RoleUser))
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationCreate(ctx, t, backend, &session, conversationSecond)
	ana, err := backend.Login(ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &ana, conversationThird)
	body := `{"items":[{"id":"` + conversationFirst + `","patch":{"pinned":true}},{"id":"` +
		conversationSecond + `","patch":{"archived":true,"title":"Old"}},{"id":"` +
		conversationThird + `","patch":{"pinned":true}}]}`
	result := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/conversations/batch", body, 207),
		webapi.DecodeBatchAnswer,
	).Results
	conversationEqual(t, len(result), 3)
	first, ok := result[0].(*webapi.BatchResultUpdated)
	if !ok {
		t.Fatal("first patch refused")
	}
	conversationEqual(t, string(first.ID), conversationFirst)
	conversationEqual(t, first.Conversation.Pinned, true)
	conversationEqual(
		t,
		first.Results,
		[]webapi.FieldResult{&webapi.FieldResultApplied{Field: webapi.PatchFieldPinned}},
	)
	second, ok := result[1].(*webapi.BatchResultUpdated)
	if !ok {
		t.Fatal("second patch refused")
	}
	conversationEqual(t, second.Conversation.Archived, true)
	conversationEqual(t, len(second.Results), 2)
	conversationEqual[webapi.FieldResult](
		t,
		second.Results[0],
		&webapi.FieldResultApplied{Field: webapi.PatchFieldArchived},
	)
	failed, ok := second.Results[1].(*webapi.FieldResultFailed)
	if !ok {
		t.Fatal("rename accepted")
	}
	conversationEqual(t, failed.Code, webapi.ErrorCodeConversationArchived)
	third, ok := result[2].(*webapi.BatchResultRefused)
	if !ok {
		t.Fatal("foreign patch accepted")
	}
	conversationEqual(t, string(third.ID), conversationThird)
	conversationEqual(t, third.Code, webapi.ErrorCodeConversationNotFound)
	conversationEqual(t, conversationSummary(ctx, t, backend, &ana, conversationThird).Pinned, false)
	items := make([]string, 101)
	for i := range items {
		items[i] = `{"id":"` + conversationFirst + `","patch":{}}`
	}
	for _, body := range []string{
		`{"items":[]}`,
		`{"items":[` + strings.Join(items, ",") + `]}`,
		`{"items":[{"id":"` + conversationFirst + `"}]}`,
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "POST", "/api/conversations/batch", body, 400),
			webapi.ErrorCodeInvalidBody,
		)
	}
}

// keyedRuntime preserves request counts until an entry revision replaces it.
type keyedRuntime struct {
	key     string
	count   int
	calls   chan keyedCall
	release <-chan struct{}
	built   *atomic.Int32
}
type keyedCall struct {
	// Key identifies the credential used for this request.
	Key string
	// Count numbers requests within one runtime revision.
	Count int
}

// Run scripts counted responses for each credential revision.
func (r *keyedRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		r.count++
		select {
		case r.calls <- keyedCall{r.key, r.count}:
		case <-ctx.Done():
			return
		}
		if r.key == "old-key" && r.count == 1 {
			select {
			case <-r.release:
			case <-ctx.Done():
				return
			}
		}
		output := uint64(0)
		if request.OutputLimit != nil {
			output = uint64(*request.OutputLimit)
		}
		for e := range providertest.Events(
			providertest.Text(fmt.Sprintf("%s:%d", r.key, r.count)),
			providertest.Response(1, output),
		)(ctx, request) {
			if !yield(e) {
				return
			}
		}
	}
}

// Fresh starts a new request count for a rebuilt runtime.
func (r *keyedRuntime) Fresh() provider.Runtime {
	r.built.Add(1)
	return &keyedRuntime{key: r.key, calls: r.calls, release: r.release, built: r.built}
}

// Close requires no resource cleanup.
func (*keyedRuntime) Close(context.Context) error { return nil }

// RequestLimits returns the fixture request limits.
func (*keyedRuntime) RequestLimits(core.Model) provider.RequestLimits {
	return provider.RequestLimits{}
}

// TestProviderEditRebuildsNextRequestAndDeletionRefusesInference checks that provider edits rebuild
// the next runtime and deletion refuses inference.
func TestProviderEditRebuildsNextRequestAndDeletionRefusesInference(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	calls := make(chan keyedCall, 4)
	release := make(chan struct{})
	var built atomic.Int32
	harness.Config.Families.Register(
		"keyed",
		conversationFamily{build: func(args providers.FamilyArgs) provider.Runtime {
			key := args.Credential.(*providers.APIKeyArgs).APIKey.Expose()
			return &keyedRuntime{key: key, calls: calls, release: release, built: &built}
		}},
	)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"custom","providerType":"keyed","label":"Keyed","apiKey":"old-key",` +
		`"models":[` + conversationConfigured(
		4000,
	) + `]}`
	entry := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/providers", body, 201),
		webapi.DecodeProviderAnswer,
	).Provider.ID
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, string(entry), "m")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m1", "before the edit")))
	var observed []keyedCall
	select {
	case call := <-calls:
		observed = append(observed, call)
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	path := "/api/providers/" + string(entry)
	conversationRequest(
		ctx,
		t,
		backend,
		&session,
		"PATCH",
		path,
		`{"apiKey":"new-key","models":[`+conversationConfigured(8000)+`]}`,
		200,
	)
	close(release)
	_, err = socket.UntilIdle(ctx)
	wireMust(t, err)
	_, err = socket.Chat(ctx, "m2", "after the edit")
	wireMust(t, err)
	_, err = socket.Chat(ctx, "m3", "the same entry")
	wireMust(t, err)
	for range 2 {
		select {
		case call := <-calls:
			observed = append(observed, call)
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	conversationEqual(t, observed, []keyedCall{{"old-key", 1}, {"new-key", 1}, {"new-key", 2}})
	conversationEqual(t, built.Load(), int32(2))
	totals := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	conversationEqual(t, totals.Totals[0].Requests, uint64(3))
	conversationEqual(t, totals.Totals[0].OutputTokens, uint64(20000))
	conversationRequest(ctx, t, backend, &session, "DELETE", path, "", 204)
	turn, err := socket.Chat(ctx, "m4", "after the deletion")
	wireMust(t, err)
	refused := false
	for _, f := range turn {
		if e, ok := f.(*framewire.ErrorFrame); ok &&
			strings.Contains(e.Message, "is no longer available to this conversation") {
			refused = true
		}
	}
	if !refused {
		t.Fatal("deleted entry did not refuse inference")
	}
	conversationEqual(t, len(calls), 0)
	totals = conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	conversationEqual(t, totals.Totals[0].Requests, uint64(3))
	kinds := conversationKinds(t, conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks)
	conversationEqual(t, kinds[len(kinds)-1], "error")
}

// TestModelSettingsReachEveryPageAndNextRequest checks that model settings reach all pages and
// subsequent requests.
func TestModelSettingsReachEveryPageAndNextRequest(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	model := func(id, tier string) string {
		return `{"id":"` + id + `","displayName":"` + strings.ToUpper(
			id,
		) + `","contextWindow":100000,"outputLimit":4000,"thinkingEfforts":["low","high"],` +
			`"acceptedExtensions":null,"fastTier":` + tier + `}`
	}
	body := `{"source":"custom","providerType":"anthropic","label":"Work",` +
		`"apiKey":"sk-ant-test","baseUrl":` + conversationJSON(
		t,
		vendor.URL("/v1"),
	) + `,"models":[` + model(
		"m",
		`"priority"`,
	) + `,` + model(
		"n",
		"null",
	) + `]}`
	entry := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/providers", body, 201),
		webapi.DecodeProviderAnswer,
	).Provider.ID
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationCreate(ctx, t, backend, &session, conversationSecond)
	settings := func(model string, effort, tier *string) *webapi.ModelSettings {
		return &webapi.ModelSettings{ProviderID: entry, ModelID: model, ThinkingEffort: effort, ServiceTierID: tier}
	}
	chosen := conversationChoose(ctx, t, backend, &session, conversationFirst, string(entry), "m")
	conversationEqual(t, chosen.Model, settings("m", nil, nil))
	other, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	socket := conversationOpen(ctx, t, backend, &other, conversationFirst)
	path := "/api/conversations/" + conversationFirst
	raised := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"thinkingEffort":"high"}`, 200),
		webapi.DecodeConversationUpdate,
	)
	conversationEqual(
		t,
		raised.Results,
		[]webapi.FieldResult{&webapi.FieldResultApplied{Field: webapi.PatchFieldThinkingEffort}},
	)
	high, low, priority := "high", "low", "priority"
	fast := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &other, "PATCH", path, `{"serviceTierId":"priority"}`, 200),
		webapi.DecodeConversationUpdate,
	)
	conversationEqual(t, fast.Conversation.Model, settings("m", &high, &priority))
	conversationEqual(
		t,
		conversationSummary(ctx, t, backend, &session, conversationFirst).Model,
		settings("m", &high, &priority),
	)
	vendor.Respond(conversationAnswer(t, []string{"one"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "first")
	wireMust(t, err)
	check := func(index int, model, effort string, tier *string) {
		fields, err := contract.Object(vendor.Requests()[index].Body)
		wireMust(t, err)
		output, err := contract.Object(fields["output_config"])
		wireMust(t, err)
		conversationEqual(t, string(fields["model"]), conversationJSON(t, model))
		conversationEqual(t, string(output["effort"]), conversationJSON(t, effort))
		if tier == nil {
			if _, ok := fields["service_tier"]; ok {
				t.Fatal("unavailable tier still sent")
			}
		} else {
			conversationEqual(t, string(fields["service_tier"]), conversationJSON(t, *tier))
		}
	}
	check(0, "m", "high", &priority)
	switched := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"PATCH",
			path,
			`{"model":{"providerId":"`+string(entry)+`","modelId":"n"},"thinkingEffort":"high"}`,
			200,
		),
		webapi.DecodeConversationUpdate,
	)
	conversationEqual(
		t,
		switched.Results,
		[]webapi.FieldResult{
			&webapi.FieldResultApplied{Field: webapi.PatchFieldModel},
			&webapi.FieldResultApplied{Field: webapi.PatchFieldThinkingEffort},
		},
	)
	conversationEqual(t, switched.Conversation.Model, settings("n", &high, nil))
	vendor.Respond(conversationAnswer(t, []string{"two"}, 1, 1))
	_, err = socket.Chat(ctx, "m2", "second")
	wireMust(t, err)
	check(1, "n", "high", nil)
	wireMust(t, socket.Send(ctx, &framewire.CloseFrame{}))
	_, err = socket.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.ClosedFrame)
		return ok
	})
	wireMust(t, err)
	conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"thinkingEffort":"low"}`, 200)
	_, err = socket.Open(ctx)
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"three"}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "third")
	wireMust(t, err)
	fields, err := contract.Object(vendor.Requests()[2].Body)
	wireMust(t, err)
	output, err := contract.Object(fields["output_config"])
	wireMust(t, err)
	conversationEqual(t, string(output["effort"]), `"low"`)
	for _, r := range []struct {
		path, body string
		status     int
		code       webapi.ErrorCode
	}{
		{path, `{"serviceTierId":"priority"}`, 409, webapi.ErrorCodeSettingUnavailable},
		{path, `{"thinkingEffort":"max"}`, 409, webapi.ErrorCodeSettingUnavailable},
		{path, `{"model":{"providerId":"` + string(entry) + `","modelId":"x"}}`, 404, webapi.ErrorCodeModelNotFound},
		{"/api/conversations/" + conversationSecond, `{"thinkingEffort":"low"}`, 409, webapi.ErrorCodeModelNotSelected},
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "PATCH", r.path, r.body, r.status),
			r.code,
		)
	}
	conversationEqual(
		t,
		conversationSummary(ctx, t, backend, &session, conversationFirst).Model,
		settings("n", &low, nil),
	)
}

// TestDeepSeekToolContinuationReplaysReasoning checks that DeepSeek tool continuations replay
// reasoning.
// A local compatible endpoint drives a real Cloud shell continuation.
func TestDeepSeekToolContinuationReplaysReasoning(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	vendor.RespondAt(
		"/api.json",
		providertest.MockResponse{
			Status:  200,
			Headers: http.Header{"Etag": {`"fixture"`}},
			Chunks: [][]byte{
				[]byte(
					`{"deepseek":{"id":"deepseek","name":"DeepSeek",` +
						`"npm":"@ai-sdk/openai-compatible","api":"https://api.deepseek.com","models":{}}}`,
				),
			},
		},
	)
	endpoint, err := url.Parse(vendor.URL("/api.json"))
	wireMust(t, err)
	harness.Config.ModelsDevURL = endpoint
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"vendor","vendorId":"deepseek","label":"DeepSeek","apiKey":"fake-key",` +
		`"baseUrl":` + conversationJSON(
		t,
		vendor.URL("/v1"),
	) + `,"models":[` + conversationConfigured(
		8000,
	) + `]}`
	entry := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "POST", "/api/providers", body, 201),
		webapi.DecodeProviderAnswer,
	).Provider.ID
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, string(entry), "m")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	delta := `{"reasoning_content":"Read the current directory.","tool_calls":[{"index":0,` +
		`"id":"call-1","function":{"name":"shell_exec","arguments":"{\"script\":\"pwd\",` +
		`\"timeoutMs\":1000}"}}]}`
	vendor.Respond(providertest.EventStream("data: {\"choices\":[{\"delta\":" + delta + "}]}\n\ndata: [DONE]\n\n"))
	vendor.Respond(
		providertest.EventStream("data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n"),
	)
	_, err = socket.Chat(ctx, "m1", "read the current directory")
	wireMust(t, err)
	var requests []providertest.RecordedRequest
	for _, r := range vendor.Requests() {
		if r.URI == "/v1/chat/completions" {
			requests = append(requests, r)
		}
	}
	conversationEqual(t, len(requests), 2)
	fields, err := contract.Object(requests[1].Body)
	wireMust(t, err)
	messages, err := contract.Decode[[]json.RawMessage](fields["messages"])
	wireMust(t, err)
	found := false
	for _, message := range messages {
		fields, err := contract.Object(message)
		wireMust(t, err)
		if _, ok := fields["tool_calls"]; ok {
			found = true
			conversationEqual(t, string(fields["reasoning_content"]), `"Read the current directory."`)
		}
	}
	if !found {
		t.Fatal("no replayed tool call")
	}
	live, err := socket.Live(ctx)
	wireMust(t, err)
	conversationEqual(t, conversationLastText(t, live), "done")
}

// TestStalledPageDoesNotHoldShutdown checks that a stalled page cannot block shutdown.
// An 8-MiB transcript fills a deliberately stalled TCP receiver; shutdown must
// cancel the blocked write, or Close never returns. Under -race this scenario
// costs about 22 seconds, mostly preparing and transferring the reply.
func TestStalledPageDoesNotHoldShutdown(t *testing.T) {
	t.Parallel()
	started := time.Now()
	ctx, harness := conversationHarness(t)
	harness.Config.Pages.CloseWait = 100 * time.Millisecond
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, entry, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{strings.Repeat("x", 8<<20)}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "Write at length.")
	wireMust(t, err)
	t.Logf("reply prepared in %s", time.Since(started))
	resetStarted := time.Now()
	stalled, err := backend.StallConversationReset(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	if stalled.ResetBytes <= 8<<20 {
		t.Fatalf("reset too small: %d", stalled.ResetBytes)
	}
	t.Logf("reset header received in %s", time.Since(resetStarted))
	closeStarted := time.Now()
	wireMust(t, backend.Close(ctx))
	t.Logf("shutdown completed in %s", time.Since(closeStarted))
}

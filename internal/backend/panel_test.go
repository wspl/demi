package backend_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/webapi"
)

// Each scenario runs one local backend without a runner or model. The HTTP
// operations protect durable ordering, bounds and ownership; budget 10 seconds.
func TestWorkPanelChangesOneOperationAtATimeAndUsesIDsOnce(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	path := "/api/conversations/" + conversationFirst + "/panel"
	read := func() webapi.WorkPanel {
		return conversationDecode(
			t,
			conversationRequest(ctx, t, backend, &session, "GET", path, "", 200),
			webapi.DecodeWorkPanel,
		)
	}
	change := func(method, route, body string, want uint64) {
		got := conversationDecode(
			t,
			conversationRequest(ctx, t, backend, &session, method, path+route, body, 200),
			webapi.DecodePanelRevision,
		)
		if got.Revision != want {
			t.Fatalf("revision got %d want %d", got.Revision, want)
		}
	}
	if got := read(); got.Revision != 0 || len(got.Tabs) != 0 {
		t.Fatalf("got %+v; want empty panel", got)
	}
	change("POST", "/tabs", `{"id":"p1","kind":"page","data":{"url":"https://a.test/"}}`, 1)
	conversationEqual(t, conversationSummary(ctx, t, backend, &session, conversationFirst).PanelRevision, uint64(1))
	change("POST", "/tabs", `{"id":"p1","kind":"page","data":{"url":"other"}}`, 1)
	change("POST", "/tabs", `{"id":"p2","kind":"page","data":{"url":"https://b.test/"},"index":0}`, 2)
	if got := read(); len(got.Tabs) != 2 || got.Tabs[0].ID != "p2" {
		t.Fatalf("got %+v; want p2,p1", got)
	}
	change("PATCH", "/tabs/p1", `{"data":{"title":"A"}}`, 3)
	if got := string(read().Tabs[1].Data); got != `{"url":"https://a.test/","title":"A"}` {
		t.Fatalf("got %s; want url and title", got)
	}
	change("PATCH", "/tabs/p1", `{"data":{"title":null}}`, 4)
	change("PATCH", "/tabs/p1", `{"data":{"title":null}}`, 4)
	change("POST", "/tabs/p1/move", `{"index":0}`, 5)
	change("POST", "/tabs/p1/move", `{"index":0}`, 5)
	change("DELETE", "/tabs/p2", "", 6)
	change("DELETE", "/tabs/p2", "", 6)
	change("POST", "/tabs", `{"id":"p2","kind":"page","data":{}}`, 6)
	change("PATCH", "/tabs/p2", `{"data":{"url":"x"}}`, 6)
	if got := read(); got.Revision != 6 || len(got.Tabs) != 1 || got.Tabs[0].ID != "p1" {
		t.Fatalf("got %+v; want revision 6 and p1", got)
	}
	for _, body := range []string{
		`{"id":"","kind":"page","data":{}}`,
		`{"id":"` + strings.Repeat("x", 65) + `","kind":"page","data":{}}`,
		`{"id":"q","kind":"page","data":[1]}`,
		`{"id":"q","kind":"page"}`,
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "POST", path+"/tabs", body, 400),
			webapi.ErrorCodeInvalidBody,
		)
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			path+"/tabs",
			`{"id":"q","kind":"unknown","data":{}}`,
			400,
		),
		webapi.ErrorCodeUnknownPanelKind,
	)
}

func TestWorkPanelBoundsAndArchivedConversation(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	path := "/api/conversations/" + conversationFirst + "/panel"
	for i := range 64 {
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			path+"/tabs",
			fmt.Sprintf(`{"id":"t%d","kind":"page","data":{"url":"https://a.test/"}}`, i),
			200,
		)
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			path+"/tabs",
			`{"id":"extra","kind":"page","data":{}}`,
			409,
		),
		webapi.ErrorCodePanelFull,
	)
	large := strings.Repeat("x", 70*1024)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &session, "PATCH", path+"/tabs/t0", `{"data":{"large":"`+large+`"}}`, 413),
		webapi.ErrorCodeTooLarge,
	)
	conversationRequest(ctx, t, backend, &session, "DELETE", path+"/tabs/t0", "", 200)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			path+"/tabs",
			`{"id":"big","kind":"page","data":{"large":"`+large+`"}}`,
			409,
		),
		webapi.ErrorCodePanelFull,
	)
	conversationRequest(
		ctx,
		t,
		backend,
		&session,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"archived":true}`,
		200,
	)
	got := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", path, "", 200),
		webapi.DecodeWorkPanel,
	)
	if len(got.Tabs) != 63 || got.Revision != 65 {
		t.Fatalf("got %d tabs revision %d; want 63 and 65", len(got.Tabs), got.Revision)
	}
	for _, change := range []struct{ method, route, body string }{
		{"POST", "/tabs", `{}`},
		{"PATCH", "/tabs/t1", `{}`},
		{"DELETE", "/tabs/t1", ""},
		{"POST", "/tabs/t1/move", `{}`},
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, change.method, path+change.route, change.body, 409),
			webapi.ErrorCodeConversationArchived,
		)
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"GET",
			"/api/conversations/"+conversationSecond+"/panel",
			"",
			404,
		),
		webapi.ErrorCodeConversationNotFound,
	)
}

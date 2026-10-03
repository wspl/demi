package backend_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/webapi"
)

// One local backend; no runner or model. Exercises whole-document panel storage.
func TestWorkPanelSavedWholeAndNeverInterpreted(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	path := "/api/conversations/" + conversationFirst + "/panel"
	read := func() webapi.WorkPanel {
		return conversationDecode(t, conversationRequest(ctx, t, b, &s, "GET", path, "", 200), webapi.DecodeWorkPanel)
	}
	decode := func(body string) webapi.WorkPanel {
		p, err := webapi.DecodeWorkPanel([]byte(body))
		wireMust(t, err)
		return p
	}
	empty := `{"selection":null,"tabs":[]}`
	conversationEqual(t, read(), decode(empty))
	panel := `{"selection":"tab-1","tabs":[{"id":"tab-1","kind":"browser","data":{"url":"about:blank"}},{"id":"tab-2","kind":"a kind the backend never heard of","data":[1,{"nested":true}]}]}`
	conversationRequest(ctx, t, b, &s, "PUT", path, panel, 204)
	conversationEqual(t, read(), decode(panel))
	conversationRequest(ctx, t, b, &s, "PUT", path, empty, 204)
	conversationEqual(t, read(), decode(empty))
	tabs := func(count int) string {
		parts := make([]string, count)
		for i := range parts {
			parts[i] = fmt.Sprintf(`{"id":"t%d","kind":"page","data":null}`, i)
		}
		return `{"selection":"change","tabs":[` + strings.Join(parts, ",") + `]}`
	}
	for _, test := range []struct {
		body   string
		status int
		code   webapi.ErrorCode
	}{
		{`{"selection":"change"}`, 400, webapi.ErrorCodeInvalidBody},
		{`{"selection":"change","tabs":[{"id":"a","kind":"page","data":null,"status":"loading"}]}`, 400, webapi.ErrorCodeInvalidBody},
		{tabs(65), 400, webapi.ErrorCodeInvalidBody},
		{`{"selection":"change","tabs":[{"id":"a","kind":"page","data":"` + strings.Repeat("x", 70000) + `"}]}`, 413, webapi.ErrorCodeTooLarge},
	} {
		conversationRefusal(t, conversationRequest(ctx, t, b, &s, "PUT", path, test.body, test.status), test.code)
	}
	conversationRequest(ctx, t, b, &s, "PUT", path, tabs(64), 204)
	conversationEqual(t, read(), decode(tabs(64)))
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "GET", "/api/conversations/"+conversationSecond+"/panel", "", 404), webapi.ErrorCodeConversationNotFound)
	conversationRequest(ctx, t, b, &s, "PATCH", "/api/conversations/"+conversationFirst, `{"archived":true}`, 200)
	conversationEqual(t, read(), decode(tabs(64)))
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "PUT", path, panel, 409), webapi.ErrorCodeConversationArchived)
}

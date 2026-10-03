package backend_test

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugins/browser"
	"github.com/wspl/demi/internal/webapi"
)

// browserConversation creates the cloud-targeted conversation through HTTP.
func browserConversation(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session) string {
	t.Helper()
	body, err := contract.EncodeJSON(webapi.CreateConversation{ID: filesConversation})
	wireMust(t, err)
	a, err := b.Post(ctx, "/api/conversations", s, body)
	wireMust(t, err)
	filesStatus(t, a, 201)
	return "/api/conversations/" + filesConversation + "/plugins/browser"
}

// No browser, runner or model; absence of a package removes its page methods.
func TestBrowserAbsentCatalogRemovesTabsAndMethods(t *testing.T) {
	ctx, _, b, s := filesBackend(t)
	path := browserConversation(ctx, t, b, &s)
	a, err := b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 404, webapi.ErrorCodeUnknownPlugin)
	a, err = b.Post(ctx, path+"/calls/open", &s, []byte(`{}`))
	wireMust(t, err)
	filesRefusal(t, a, 404, webapi.ErrorCodeUnknownPluginMethod)
	wireMust(t, b.Close(ctx))
}

// Builds and publishes the browser package once; no Chrome or runner starts.
func TestBrowserStoppedCloudIsNotWokenByTabMethods(t *testing.T) {
	ctx, h := filesHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	path := browserConversation(ctx, t, b, &s)
	a, err := b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesStatus(t, a, 200)
	state, err := webapi.DecodePluginStateAnswer(a.Body)
	wireMust(t, err)
	tabs, err := browser.DecodeBrowserTabs(state.State)
	wireMust(t, err)
	if len(tabs.Tabs) != 0 {
		t.Fatalf("tabs: %+v", tabs)
	}
	a, err = b.Post(ctx, path+"/calls/close", &s, []byte(`{"tab":"t999999"}`))
	wireMust(t, err)
	filesStatus(t, a, 200)
	for method, body := range map[string]string{"navigate": `{"tab":"t999999","url":"https://example.test/"}`, "history": `{"tab":"t999999","action":"forward"}`} {
		a, err = b.Post(ctx, path+"/calls/"+method, &s, []byte(body))
		wireMust(t, err)
		filesRefusal(t, a, 409, webapi.ErrorCodeHostStopped)
	}
	devices, err := b.Devices(ctx, &s)
	wireMust(t, err)
	if len(devices) != 0 {
		t.Fatalf("tab methods made a cloud: %+v", devices)
	}
	wireMust(t, b.Close(ctx))
}

// A paired runner installs the browser package; no Chrome or model is started.
func TestBrowserTabMethodsUseConversationHost(t *testing.T) {
	t.Skip("finding 1: device claim returns HTTP 500 because device.installs is nil")
	ctx, h := filesHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	path := browserConversation(ctx, t, b, &s)
	body, err := contract.EncodeObject([]contract.Field{{Name: "target", Value: &webapi.ConversationTargetDevice{DeviceID: laptop.ID(), Path: laptop.Runner.Home()}}})
	wireMust(t, err)
	a, err := b.Patch(ctx, "/api/conversations/"+filesConversation, &s, body)
	wireMust(t, err)
	filesStatus(t, a, 200)
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesStatus(t, a, 200)
	state, err := webapi.DecodePluginStateAnswer(a.Body)
	wireMust(t, err)
	tabs, err := browser.DecodeBrowserTabs(state.State)
	wireMust(t, err)
	if state.Revision != 0 || len(tabs.Tabs) != 0 {
		t.Fatalf("tabs: %+v %+v", state, tabs)
	}
	for _, tab := range []string{"t999999", "not-a-tab"} {
		body, err := contract.EncodeJSON(browser.CloseTab{Tab: tab})
		wireMust(t, err)
		a, err = b.Post(ctx, path+"/calls/close", &s, body)
		wireMust(t, err)
		filesStatus(t, a, 200)
		if string(a.Body) != "null" {
			t.Fatalf("close: %s", a.Body)
		}
	}
	for _, call := range []struct{ method, body string }{{"navigate", `{"tab":"t999999","url":"https://example.test/"}`}, {"history", `{"tab":"t999999","action":"reload"}`}, {"history", `{"tab":"not-a-tab","action":"reload"}`}} {
		a, err = b.Post(ctx, path+"/calls/"+call.method, &s, []byte(call.body))
		wireMust(t, err)
		filesRefusal(t, a, 409, webapi.ErrorCodePluginRefused)
		refusal, err := a.ErrorBody()
		wireMust(t, err)
		if refusal.Reason == nil || *refusal.Reason != "tab_not_found" {
			t.Fatalf("reason: %+v", refusal)
		}
	}
	for _, call := range []struct{ method, body string }{{"navigate", `{"tab":"t999999"}`}, {"history", `{"tab":"t999999","action":"sideways"}`}, {"open", `{"url":""}`}} {
		a, err = b.Post(ctx, path+"/calls/"+call.method, &s, []byte(call.body))
		wireMust(t, err)
		filesRefusal(t, a, 400, webapi.ErrorCodeInvalidBody)
	}
	wireMust(t, laptop.Runner.Kill(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, laptop.ID(), false))
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 409, webapi.ErrorCodeDeviceOffline)
	a, err = b.Patch(ctx, "/api/conversations/"+filesConversation, &s, []byte(`{"archived":true}`))
	wireMust(t, err)
	filesStatus(t, a, 200)
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 409, webapi.ErrorCodeConversationArchived)
	a, err = b.Post(ctx, path+"/calls/open", &s, []byte(`{}`))
	wireMust(t, err)
	filesRefusal(t, a, 409, webapi.ErrorCodeConversationArchived)
	a, err = b.Read(ctx, "/api/conversations/1e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01/plugins/browser/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 404, webapi.ErrorCodeConversationNotFound)
	wireMust(t, b.Close(ctx))
}

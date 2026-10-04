package scenarios_test

import (
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/plugin/browser"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestBrowserAbsentCatalogRemovesTabsAndMethods uses no browser, runner or model to observe a missing package’s
// page methods.
func TestBrowserAbsentCatalogRemovesTabsAndMethods(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, filesConversation)
	path := "/api/conversations/" + filesConversation + "/plugins/browser"
	a, err := b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 404, webapiproto.ErrorCodeUnknownPlugin)
	a, err = b.Post(ctx, path+"/calls/bind", &s, []byte(`{"panelTab":"a"}`))
	wireMust(t, err)
	filesRefusal(t, a, 404, webapiproto.ErrorCodeUnknownPluginMethod)
	wireMust(t, b.Close(ctx))
}

// TestBrowserStoppedCloudIsNotWokenByTabMethods builds and publishes the browser package once; no Chrome or
// runner starts.
func TestBrowserStoppedCloudIsNotWokenByTabMethods(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, filesConversation)
	path := "/api/conversations/" + filesConversation + "/plugins/browser"
	a, err := b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesStatus(t, a, 200)
	state, err := webapiproto.DecodePluginStateAnswer(a.Body)
	wireMust(t, err)
	tabs, err := browser.DecodeBrowserTabs(state.State)
	wireMust(t, err)
	if len(tabs.Tabs) != 0 {
		t.Fatalf("tabs: %+v", tabs)
	}
	a, err = b.Post(ctx, path+"/calls/sync", &s, []byte(`{}`))
	wireMust(t, err)
	filesStatus(t, a, 200)
	for method, body := range map[string]string{
		"navigate": `{"tab":"t999999","url":"https://example.test/"}`,
		"history":  `{"tab":"t999999","action":"forward"}`,
	} {
		a, err = b.Post(ctx, path+"/calls/"+method, &s, []byte(body))
		wireMust(t, err)
		filesRefusal(t, a, 409, webapiproto.ErrorCodeHostStopped)
	}
	devices, err := b.Devices(ctx, &s)
	wireMust(t, err)
	if len(devices) != 0 {
		t.Fatalf("tab methods made a cloud: %+v", devices)
	}
	wireMust(t, b.Close(ctx))
}

// TestBrowserTabMethodsUseConversationHost uses a paired runner to install the browser package; no Chrome or
// model is started.
func TestBrowserTabMethodsUseConversationHost(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, filesConversation)
	path := "/api/conversations/" + filesConversation + "/plugins/browser"
	body, err := contract.EncodeObject(
		[]contract.Field{
			{
				Name:  "target",
				Value: &webapiproto.ConversationTargetDevice{DeviceID: laptop.ID(), Path: laptop.Runner.Home()},
			},
		},
	)
	wireMust(t, err)
	a, err := b.Patch(ctx, "/api/conversations/"+filesConversation, &s, body)
	wireMust(t, err)
	filesStatus(t, a, 200)
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesStatus(t, a, 200)
	state, err := webapiproto.DecodePluginStateAnswer(a.Body)
	wireMust(t, err)
	tabs, err := browser.DecodeBrowserTabs(state.State)
	wireMust(t, err)
	if state.Revision != 0 || len(tabs.Tabs) != 0 {
		t.Fatalf("tabs: %+v %+v", state, tabs)
	}
	a, err = b.Post(ctx, path+"/calls/sync", &s, []byte(`{}`))
	wireMust(t, err)
	filesStatus(t, a, 200)
	for _, call := range []struct {
		method, body string
	}{
		{
			"navigate",
			`{"tab":"t999999","url":"https://example.test/"}`,
		},
		{
			"history",
			`{"tab":"t999999","action":"reload"}`,
		},
		{
			"history",
			`{"tab":"not-a-tab","action":"reload"}`,
		},
	} {
		a, err = b.Post(ctx, path+"/calls/"+call.method, &s, []byte(call.body))
		wireMust(t, err)
		filesRefusal(t, a, 409, webapiproto.ErrorCodePluginRefused)
		refusal, err := a.ErrorBody()
		wireMust(t, err)
		if refusal.Reason == nil || *refusal.Reason != "tab_not_found" {
			t.Fatalf("reason: %+v", refusal)
		}
	}
	for _, call := range []struct {
		method, body string
	}{
		{
			"navigate",
			`{"tab":"t999999"}`,
		},
		{
			"history",
			`{"tab":"t999999","action":"sideways"}`,
		},
		{
			"bind",
			`{}`,
		},
	} {
		a, err = b.Post(ctx, path+"/calls/"+call.method, &s, []byte(call.body))
		wireMust(t, err)
		filesRefusal(t, a, 400, webapiproto.ErrorCodeInvalidBody)
	}
	wireMust(t, laptop.Runner.Kill(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, laptop.ID(), false))
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 409, webapiproto.ErrorCodeDeviceOffline)
	a, err = b.Patch(ctx, "/api/conversations/"+filesConversation, &s, []byte(`{"archived":true}`))
	wireMust(t, err)
	filesStatus(t, a, 200)
	a, err = b.Read(ctx, path+"/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 409, webapiproto.ErrorCodeConversationArchived)
	a, err = b.Post(ctx, path+"/calls/navigate", &s, []byte(`{"tab":"t999999","url":"https://example.test/"}`))
	wireMust(t, err)
	filesRefusal(t, a, 409, webapiproto.ErrorCodeConversationArchived)
	a, err = b.Read(ctx, "/api/conversations/1e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01/plugins/browser/state", &s)
	wireMust(t, err)
	filesRefusal(t, a, 404, webapiproto.ErrorCodeConversationNotFound)
	wireMust(t, b.Close(ctx))
}

// TestBrowserJobsAndTabMethodsRaiseSummaryRevisions uses a real shell completion and a user tab operation to
// publish independent revisions.
func TestBrowserJobsAndTabMethodsRaiseSummaryRevisions(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-browser")
	fresh := conversationSummary(w.ctx, t, w.backend, &w.session, filesConversation)
	conversationEqual(t, fresh.WorkingTreeRevision, uint64(0))
	conversationEqual(t, fresh.PluginRevisions, []webapiproto.PluginRevision{{Plugin: "browser", Revision: 0}})
	w.vendor.Respond(conversationShell(t, "t1", "true", 30000))
	w.vendor.Respond(conversationAnswer(t, []string{"done"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	ended := conversationSummary(w.ctx, t, w.backend, &w.session, filesConversation)
	conversationEqual(t, ended.WorkingTreeRevision, uint64(1))
	conversationEqual(t, ended.PluginRevisions, []webapiproto.PluginRevision{{Plugin: "browser", Revision: 1}})
	path := "/api/conversations/" + filesConversation + "/plugins/browser"
	state := conversationDecode(
		t,
		conversationRequest(w.ctx, t, w.backend, &w.session, "GET", path+"/state", "", 200),
		webapiproto.DecodePluginStateAnswer,
	)
	conversationEqual(t, state.Revision, uint64(1))
	tabs, err := browser.DecodeBrowserTabs(state.State)
	wireMust(t, err)
	conversationEqual(t, len(tabs.Tabs), 0)
	conversationRequest(w.ctx, t, w.backend, &w.session, "POST", path+"/calls/sync", `{}`, 200)
	operated := conversationSummary(w.ctx, t, w.backend, &w.session, filesConversation)
	conversationEqual(t, operated.WorkingTreeRevision, uint64(1))
	conversationEqual(t, operated.PluginRevisions, []webapiproto.PluginRevision{{Plugin: "browser", Revision: 2}})
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestBrowserListingRunningCloudDoesNotKeepItAwake reads real tabs repeatedly while the cloud's idle watch stops
// its runner.
func TestBrowserListingRunningCloudDoesNotKeepItAwake(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	h, manager, err := backendtest.HostsHarness(ctx, t)
	wireMust(t, err)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	window := 800 * time.Millisecond
	h.Config.Lifecycle.IdleWindow = window
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, filesConversation)
	conversationRequest(ctx, t, b, &s, "GET", "/api/conversations/"+filesConversation+"/fs", "", 200)
	gate, err := backendtest.FileGate(ctx, b.Backend, s.User.ID, webapiproto.ConversationID(filesConversation))
	wireMust(t, err)
	working, err := gate.Enter(ctx, gates.Demand)
	wireMust(t, err)
	defer working.Release()
	conversationRequest(
		ctx,
		t,
		b,
		&s,
		"GET",
		"/api/conversations/"+filesConversation+"/plugins/browser/state",
		"",
		200,
	)
	devices := manager.Devices()
	conversationEqual(t, len(devices), 1)
	rested := time.Now()
	working.Release()
	hibernate := "hibernate:" + string(devices[0])
	for {
		answer, err := b.Read(ctx, "/api/conversations/"+filesConversation+"/plugins/browser/state", &s)
		// Hibernation ends this scenario. A request it overtakes may lose its
		// pipe after admission, so only validate replies before that event.
		if manager.Count(hibernate) != 0 {
			break
		}
		wireMust(t, err)
		if answer.Status != 200 {
			filesRefusal(t, answer, 409, webapiproto.ErrorCodeDeviceOffline)
		}
	}
	at, err := manager.Arrival(ctx, hibernate)
	wireMust(t, err)
	if at.Before(rested.Add(window)) {
		t.Fatalf("stopped after %s", at.Sub(rested))
	}
	wireMust(t, b.Close(ctx))
}

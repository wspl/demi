package backend_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// TestDisabledPluginLeavesPageAndRefusesCallsUntilEnabled uses local HTTP and page synchronization only; no
// runner or vendor is needed.
func TestDisabledPluginLeavesPageAndRefusesCallsUntilEnabled(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	change := func(id string, enabled bool) {
		t.Helper()
		body, err := contract.EncodeJSON(webapi.PluginSwitch{Enabled: enabled})
		wireMust(t, err)
		a, err := b.Put(ctx, "/api/plugins/"+id, &master, body)
		wireMust(t, err)
		filesStatus(t, a, 204)
	}
	renew := func() (int, webapi.ErrorBody) {
		t.Helper()
		a, err := b.Post(
			ctx,
			"/api/plugins/expose/calls/renew",
			&master,
			[]byte(`{"expose":"k7x2maqw4p3s6tavaw2y4z6aab"}`),
		)
		wireMust(t, err)
		body, err := a.ErrorBody()
		wireMust(t, err)
		return a.Status, body
	}
	change("expose", false)
	page, err := b.Sync(ctx, t, &master)
	wireMust(t, err)
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	if _, ok := state.PluginStates["expose"]; ok {
		t.Fatal("disabled expose still on page")
	}
	found := false
	for _, p := range state.Plugins {
		if p.ID == "expose" {
			found = true
			if p.Enabled {
				t.Fatal("expose still enabled")
			}
		}
	}
	if !found {
		t.Fatal("expose missing from plugin list")
	}
	status, body := renew()
	if status != 409 || body.Code != webapi.ErrorCodePluginDisabled {
		t.Fatalf("disabled call: %d %+v", status, body)
	}
	change("expose", true)
	again, err := b.Sync(ctx, t, &master)
	wireMust(t, err)
	state, err = again.Snapshot(ctx)
	wireMust(t, err)
	if _, ok := state.PluginStates["expose"]; !ok {
		t.Fatal("enabled expose absent from page")
	}
	_, body = renew()
	if body.Reason == nil || *body.Reason != "expose_not_found" {
		t.Fatalf("renew: %+v", body)
	}
	a, err := b.Put(ctx, "/api/plugins/nope", &master, []byte(`{"enabled":false}`))
	wireMust(t, err)
	filesRefusal(t, a, 404, webapi.ErrorCodeUnknownPlugin)
	wireMust(t, page.Close(ctx))
	wireMust(t, again.Close(ctx))
	wireMust(t, b.Close(ctx))
}

// TestOpenConversationKeepsPluginCommandsUntilReload uses three local scripted turns to show that command
// changes take effect only on reload.
func TestOpenConversationKeepsPluginCommandsUntilReload(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, filesConversation)
	conversationChoose(ctx, t, b, &s, filesConversation, entry, "claude-opus-4-8")
	page, _ := conversationPage(ctx, t, b, &s)
	socket := conversationOpen(ctx, t, b, &s, filesConversation)
	vendor.Respond(conversationAnswer(t, []string{"one"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "first")
	wireMust(t, err)
	system := func(index int) string {
		fields, err := contract.Object(vendor.Requests()[index].Body)
		wireMust(t, err)
		return string(fields["system"])
	}
	if !strings.Contains(system(0), "demi todo") {
		t.Fatal(system(0))
	}
	conversationRequest(ctx, t, b, &s, "PUT", "/api/plugins/todo", `{"enabled":false}`, 204)
	changes, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
		_, ok := e.(*webapi.SyncEventPlugins)
		return ok
	})
	wireMust(t, err)
	changed := false
	for _, e := range changes {
		if c, ok := e.(*webapi.SyncEventConversation); ok && c.Conversation.PluginsChanged {
			changed = true
		}
	}
	if !changed {
		t.Fatal("plugin list arrived without changed conversation summary")
	}
	found := false
	for _, p := range changes[len(changes)-1].(*webapi.SyncEventPlugins).Plugins {
		if p.ID == "todo" {
			found = true
			conversationEqual(t, p.Enabled, false)
		}
	}
	if !found {
		t.Fatal("todo missing")
	}
	vendor.Respond(conversationAnswer(t, []string{"two"}, 1, 1))
	_, err = socket.Chat(ctx, "m2", "second")
	wireMust(t, err)
	if !strings.Contains(system(1), "demi todo") {
		t.Fatal(system(1))
	}
	conversationRequest(ctx, t, b, &s, "POST", "/api/conversations/"+filesConversation+"/reload", `{}`, 204)
	_, err = socket.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.ClosedFrame)
		return ok
	})
	wireMust(t, err)
	_, err = page.Until(ctx, func(e webapi.SyncEvent) bool {
		c, ok := e.(*webapi.SyncEventConversation)
		return ok && !c.Conversation.PluginsChanged
	})
	wireMust(t, err)
	socket = conversationOpen(ctx, t, b, &s, filesConversation)
	vendor.Respond(conversationAnswer(t, []string{"three"}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "third")
	wireMust(t, err)
	if strings.Contains(system(2), "demi todo") || !strings.Contains(system(2), "demi host") {
		t.Fatal(system(2))
	}
	held := providertest.EventStream(": thinking\n\n")
	held.Ending = providertest.Open
	vendor.Respond(held)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m4", "fourth")))
	_, err = socket.Until(ctx, func(f framewire.ServerFrame) bool {
		p, ok := f.(*framewire.PhaseFrame)
		return ok && p.Phase != core.SessionPhaseIdle
	})
	wireMust(t, err)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, b, &s, "POST", "/api/conversations/"+filesConversation+"/reload", `{}`, 409),
		webapi.ErrorCodeTurnInFlight,
	)
	wireMust(t, socket.Stop(ctx))
	wireMust(t, page.Close(ctx))
	wireMust(t, socket.Close(ctx))
	wireMust(t, b.Close(ctx))
}

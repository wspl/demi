package backend_test

import (
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// Local HTTP and page synchronization only; no runner or vendor is needed.
func TestDisabledPluginLeavesPageAndRefusesCallsUntilEnabled(t *testing.T) {
	ctx, _, b, master := filesBackend(t)
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
		a, err := b.Post(ctx, "/api/plugins/expose/calls/renew", &master, []byte(`{"expose":"k7x2maqw4p3s6tavaw2y4z6aab"}`))
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
	wireMust(t, b.Close(ctx))
}

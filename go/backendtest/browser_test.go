package backendtest_test

import (
	"net/http"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// absentTab is a tab id the browser never gave out.
const absentTab = "t999999"

func tabsRoute(id string) string {
	return "/api/conversations/" + id + "/browser/tabs"
}

// Cost: one backend and a real runner, over a second: a real device installs
// the builtin package, whose browser service answers the routes. No browser
// runs, so no Chrome is needed.
func TestTheTabRoutesRunTheBrowsersOperationsAsTheUserOnTheConversationsHost(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	laptop := b.Pair(master, "laptop")
	id := newID()
	b.CreateConversation(master, id)
	b.Patch("/api/conversations/"+id, master, backendtest.Map{
		"target": backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": laptop.Home()},
	}).Expect(http.StatusOK)

	// A browser that does not run has no tabs, and nothing to close.
	backendtest.AssertJSON(t, b.Get(tabsRoute(id), master).Expect(http.StatusOK).Value(), backendtest.Map{"tabs": []any{}})
	b.Delete(tabsRoute(id)+"/"+absentTab, master).Expect(http.StatusNoContent)
	b.Delete(tabsRoute(id)+"/not-a-tab", master).Expect(http.StatusNoContent)
	// A tab the browser does not have is the browser's answer.
	navigate := tabsRoute(id) + "/" + absentTab + "/navigate"
	wantRefusal(t, b.Post(navigate, master, backendtest.Map{"url": "https://example.test/"}), http.StatusNotFound, "tab_not_found", "navigating an absent tab")
	history := tabsRoute(id) + "/" + absentTab + "/history"
	wantRefusal(t, b.Post(history, master, backendtest.Map{"action": "reload"}), http.StatusNotFound, "tab_not_found", "reloading an absent tab")
	wantRefusal(t, b.Post(tabsRoute(id)+"/not-a-tab/history", master, backendtest.Map{"action": "back"}), http.StatusNotFound, "tab_not_found", "history of a malformed tab")
	for _, refusal := range []struct {
		path string
		body backendtest.Map
	}{
		{navigate, backendtest.Map{}},
		{history, backendtest.Map{"action": "sideways"}},
		{tabsRoute(id), backendtest.Map{"url": ""}},
	} {
		wantRefusal(t, b.Post(refusal.path, master, refusal.body), http.StatusBadRequest, "invalid_body", refusal.path+" "+jsonText(refusal.body))
	}

	// Without its runner the device answers nothing.
	laptop.Runner.Kill()
	b.UntilOnline(master, laptop.ID(), false)
	wantRefusal(t, b.Get(tabsRoute(id), master), http.StatusConflict, "device_offline", "the tabs of an offline device")

	// An archived conversation's browser is not operated.
	b.Patch("/api/conversations/"+id, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	wantRefusal(t, b.Get(tabsRoute(id), master), http.StatusConflict, "conversation_archived", "the tabs of an archived conversation")
	wantRefusal(t, b.Post(tabsRoute(id), master, backendtest.Map{}), http.StatusConflict, "conversation_archived", "opening a tab of an archived conversation")
	wantRefusal(t, b.Get(tabsRoute(newID()), master), http.StatusNotFound, "conversation_not_found", "the tabs of an unknown conversation")
	b.Stop()
}

// Cost: one backend, about a second.
func TestAStoppedCloudIsNotWokenToListCloseOrMoveItsTabs(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	// A new conversation works on the Cloud, which never started.
	id := newID()
	b.CreateConversation(master, id)
	backendtest.AssertJSON(t, b.Get(tabsRoute(id), master).Expect(http.StatusOK).Value(), backendtest.Map{"tabs": []any{}})
	b.Delete(tabsRoute(id)+"/"+absentTab, master).Expect(http.StatusNoContent)
	navigate := tabsRoute(id) + "/" + absentTab + "/navigate"
	wantRefusal(t, b.Post(navigate, master, backendtest.Map{"url": "https://example.test/"}), http.StatusConflict, "host_stopped", "navigating on a stopped Cloud")
	history := tabsRoute(id) + "/" + absentTab + "/history"
	wantRefusal(t, b.Post(history, master, backendtest.Map{"action": "forward"}), http.StatusConflict, "host_stopped", "history on a stopped Cloud")
	// Looking made no Cloud.
	backendtest.AssertJSON(t, b.Get("/api/devices", master).At("devices"), []any{})
	b.Stop()
}

// Cost: one backend, about a second.
func TestABackendWhoseCatalogServesNoBrowserHasNoTabRoutes(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	id := newID()
	b.CreateConversation(master, id)
	wantRefusal(t, b.Get(tabsRoute(id), master), http.StatusNotFound, "not_found", "the tabs of a backend without a browser")
	b.Stop()
}

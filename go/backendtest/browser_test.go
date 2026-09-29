package backendtest_test

import (
	"fmt"
	"net/http"
	"testing"
	"time"

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

// Cost: one backend and a Cloud's runner, over a second: the Cloud installs the
// builtin package, and the idle window passes in real time.
func TestListingARunningCloudsTabsDoesNotKeepItAwake(t *testing.T) {
	t.Parallel()
	// Far longer than the time between two listings, so that listings counted as
	// activity would keep the Cloud up.
	const window = 800 * time.Millisecond
	h := backendtest.New(t, backendtest.WithBuiltin())
	idleAfter(h, window)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	id := newID()
	b.CreateConversation(master, id)
	// Reading the conversation's files wakes the Cloud it works on.
	b.Get("/api/conversations/"+id+"/fs", master).Expect(http.StatusOK)
	device := theCloud(t, h)
	// Closing a tab starts the browser's service, which the Cloud's runner
	// installs first. The close restarts the window as it is admitted and is no
	// activity after that, so a lease of the conversation's file gate, which is
	// its work, keeps the Cloud up meanwhile.
	working := b.Control.EnterGate(master.User.ID, id, "demand")
	b.Delete(tabsRoute(id)+"/"+absentTab, master).Expect(http.StatusNoContent)
	rested := time.Now()
	working.Release()
	// The page lists the tabs again and again, and the Cloud idles and stops all
	// the same, a window after the last activity. A listing the stop overtakes
	// finds the runner gone.
	done := make(chan struct{})
	listing := make(chan error, 1)
	go func() {
		defer close(listing)
		for {
			select {
			case <-done:
				return
			default:
			}
			listed, err := b.TryDo(backendtest.Request{Path: tabsRoute(id), Session: master})
			if err != nil {
				listing <- err
				return
			}
			if listed.Status != http.StatusOK && !(listed.Status == http.StatusConflict && listed.ErrorCode() == "device_offline") {
				listing <- fmt.Errorf("a listing is answered %d %s", listed.Status, listed.Body)
				return
			}
			time.Sleep(20 * time.Millisecond)
		}
	}()
	stopped := h.Manager.Arrival("hibernate:"+device, backendtest.Patience)
	close(done)
	if err := <-listing; err != nil {
		t.Fatal(err)
	}
	if stopped.Before(rested.Add(window)) {
		t.Fatalf("the Cloud stopped %v after the last activity", stopped.Sub(rested))
	}
	b.Stop()
}

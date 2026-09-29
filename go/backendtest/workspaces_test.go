package backendtest_test

import (
	"net/http"
	"os"
	"slices"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

func createWorkspace(t *testing.T, b *backendtest.Backend, session *backendtest.Session, body backendtest.Map) string {
	t.Helper()
	return b.Post("/api/workspaces", session, body).Expect(http.StatusCreated).Str("workspace.id")
}

// workspaceNames lists the names of the session's workspaces in their order.
func workspaceNames(t *testing.T, b *backendtest.Backend, session *backendtest.Session) []string {
	t.Helper()
	var names []string
	listed, _ := b.Get("/api/workspaces", session).At("workspaces").([]any)
	for _, workspace := range listed {
		names = append(names, backendtest.At(workspace, "name").(string))
	}
	return names
}

// createConversation creates a conversation under a new id, and answers it.
func createConversation(t *testing.T, b *backendtest.Backend, session *backendtest.Session) string {
	t.Helper()
	id := newID()
	b.Post("/api/conversations", session, backendtest.Map{"id": id}).Expect(http.StatusCreated)
	return id
}

// conversationIDs lists the ids of the session's conversations, from the path,
// in their order.
func conversationIDs(t *testing.T, b *backendtest.Backend, session *backendtest.Session, path string) []string {
	t.Helper()
	var ids []string
	listed, _ := b.Get(path, session).At("conversations").([]any)
	for _, conversation := range listed {
		ids = append(ids, backendtest.At(conversation, "id").(string))
	}
	return ids
}

func reorder(b *backendtest.Backend, session *backendtest.Session, kind, id string, before any) int {
	return b.Post("/api/sidebar/reorder", session, backendtest.Map{"kind": kind, "id": id, "beforeId": before}).Status
}

// Cost: one backend and a real runner, about a second.
func TestAWorkspacePointsAtADirectoryOfTheUsersDeviceAndStaysWhileConversationsTargetIt(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	home := laptop.Home()
	notes := createWorkspace(t, b, master, backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": home, "name": "  notes  "})
	createWorkspace(t, b, master, backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": home + "/site", "name": "site"})
	if names := workspaceNames(t, b, master); !slices.Equal(names, []string{"notes", "site"}) {
		t.Fatalf("the workspaces are %v", names)
	}
	// The product state carries them, in the same order.
	state := b.Sync(master).Snapshot()
	if backendtest.At(state, "workspaces.0.id") != notes || backendtest.At(state, "workspaces.0.path") != home {
		t.Fatalf("the snapshot's workspaces are %v", state["workspaces"])
	}

	// What a creation names must be there and the user's.
	for _, refusal := range []struct {
		body   backendtest.Map
		status int
		code   string
	}{
		{backendtest.Map{"kind": "device", "deviceId": "nothing", "path": home, "name": "x"}, http.StatusNotFound, "device_not_found"},
		{backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": "relative", "name": "x"}, http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": home, "name": "   "}, http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"deviceId": laptop.ID(), "path": home, "name": "untagged"}, http.StatusBadRequest, "invalid_body"},
	} {
		wantRefusal(t, b.Post("/api/workspaces", master, refusal.body), refusal.status, refusal.code, "the creation")
	}

	renamed := b.Patch("/api/workspaces/"+notes, master, backendtest.Map{"name": "journal"}).Expect(http.StatusOK)
	if renamed.Str("workspace.name") != "journal" {
		t.Fatalf("the renamed workspace is %s", renamed.Body)
	}
	wantRefusal(t, b.Patch("/api/workspaces/nothing", master, backendtest.Map{"name": "x"}), http.StatusNotFound, "workspace_not_found", "renaming a missing workspace")

	// A conversation targets it: neither it nor its device goes.
	conversation := createConversation(t, b, master)
	target := backendtest.Map{"target": backendtest.Map{"kind": "workspace", "workspaceId": notes}}
	b.Patch("/api/conversations/"+conversation, master, target).Expect(http.StatusOK)
	if cwd := b.Get("/api/conversations", master).At("conversations.0.cwd"); cwd != home {
		t.Fatalf("the conversation works in %v, not %s", cwd, home)
	}
	wantRefusal(t, b.Delete("/api/workspaces/"+notes, master), http.StatusConflict, "workspace_in_use", "deleting a targeted workspace")
	wantRefusal(t, b.Delete("/api/devices/"+laptop.ID(), master), http.StatusConflict, "device_in_use", "revoking a device a workspace names")

	// Once nothing targets it, it goes, and its files stay.
	away := backendtest.Map{"target": backendtest.Map{"kind": "cloud"}}
	b.Patch("/api/conversations/"+conversation, master, away).Expect(http.StatusOK)
	b.Delete("/api/workspaces/"+notes, master).Expect(http.StatusNoContent)
	if names := workspaceNames(t, b, master); !slices.Equal(names, []string{"site"}) {
		t.Fatalf("the workspaces are %v", names)
	}
	if info, err := os.Stat(home); err != nil || !info.IsDir() {
		t.Fatalf("the workspace's directory went: %v", err)
	}
	wantRefusal(t, b.Delete("/api/workspaces/"+notes, master), http.StatusNotFound, "workspace_not_found", "deleting a deleted workspace")
	b.Stop()
}

// Cost: one backend started twice and a real runner, about two seconds.
func TestTheSidebarKeepsTheUsersOrderWithinEachPartitionThroughPatchesAndARestart(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	order := func(path string) []string { return conversationIDs(t, b, master, path) }
	// New conversations enter at the front.
	a, bb, c := createConversation(t, b, master), createConversation(t, b, master), createConversation(t, b, master)
	if got := order("/api/conversations"); !slices.Equal(got, []string{c, bb, a}) {
		t.Fatalf("the order is %v", got)
	}
	if status := reorder(b, master, "conversation", a, c); status != http.StatusNoContent {
		t.Fatalf("a reorder answers %d", status)
	}
	if got := order("/api/conversations"); !slices.Equal(got, []string{a, c, bb}) {
		t.Fatalf("the order is %v", got)
	}
	// A rename changes no position, and null moves a row to the end.
	b.Patch("/api/conversations/"+a, master, backendtest.Map{"title": "kept title"}).Expect(http.StatusOK)
	if status := reorder(b, master, "conversation", c, nil); status != http.StatusNoContent {
		t.Fatalf("a move to the end answers %d", status)
	}
	if got := order("/api/conversations"); !slices.Equal(got, []string{a, bb, c}) {
		t.Fatalf("the order is %v", got)
	}

	// A pinned row is another partition: nothing moves across it.
	b.Patch("/api/conversations/"+bb, master, backendtest.Map{"pinned": true}).Expect(http.StatusOK)
	if got := order("/api/conversations"); !slices.Equal(got, []string{bb, a, c}) {
		t.Fatalf("the order is %v", got)
	}
	if status := reorder(b, master, "conversation", a, bb); status != http.StatusConflict {
		t.Fatalf("a move across a pin answers %d", status)
	}
	// So is a workspace, and an archived row is in none.
	laptop := b.Pair(master, "laptop")
	home := laptop.Home()
	notes := createWorkspace(t, b, master, backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": home, "name": "notes"})
	target := backendtest.Map{"target": backendtest.Map{"kind": "workspace", "workspaceId": notes}}
	b.Patch("/api/conversations/"+c, master, target).Expect(http.StatusOK)
	if status := reorder(b, master, "conversation", a, c); status != http.StatusConflict {
		t.Fatalf("a move across a workspace answers %d", status)
	}
	b.Patch("/api/conversations/"+a, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	refused := b.Post("/api/sidebar/reorder", master, backendtest.Map{"kind": "conversation", "id": a, "beforeId": nil})
	wantRefusal(t, refused, http.StatusConflict, "invalid_order", "the reorder of an archived row")
	unknown := b.Post("/api/sidebar/reorder", master, backendtest.Map{"kind": "project", "id": a, "beforeId": nil})
	wantRefusal(t, unknown, http.StatusBadRequest, "invalid_body", "the reorder of an unknown kind")

	// Workspaces keep their order too: a new one appends.
	site := createWorkspace(t, b, master, backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": home + "/site", "name": "site"})
	if names := workspaceNames(t, b, master); !slices.Equal(names, []string{"notes", "site"}) {
		t.Fatalf("the workspaces are %v", names)
	}
	if status := reorder(b, master, "workspace", site, notes); status != http.StatusNoContent {
		t.Fatalf("a workspace reorder answers %d", status)
	}
	if names := workspaceNames(t, b, master); !slices.Equal(names, []string{"site", "notes"}) {
		t.Fatalf("the workspaces are %v", names)
	}

	// The order is a record: it outlives the process. The laptop's runner stops
	// first, since it would keep calling the old address.
	laptop.Runner.Stop()
	b.Stop()
	b = h.Start()
	if got := order("/api/conversations?archived=false"); !slices.Equal(got, []string{bb, c}) {
		t.Fatalf("the order after a restart is %v", got)
	}
	if names := workspaceNames(t, b, master); !slices.Equal(names, []string{"site", "notes"}) {
		t.Fatalf("the workspaces after a restart are %v", names)
	}
	b.Stop()
}

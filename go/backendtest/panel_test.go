package backendtest_test

import (
	"crypto/rand"
	"fmt"
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// newID returns a fresh UUID, as a page names a conversation it creates.
func newID() string {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err != nil {
		panic(err)
	}
	bytes[6] = bytes[6]&0x0f | 0x40
	bytes[8] = bytes[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", bytes[0:4], bytes[4:6], bytes[6:8], bytes[8:10], bytes[10:])
}

// Cost: one backend, about a second.
func TestAWorkPanelIsSavedWholeReadBackAsSavedAndNeverInterpreted(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	id := newID()
	b.Post("/api/conversations", master, backendtest.Map{"id": id}).Expect(http.StatusCreated)
	path := "/api/conversations/" + id + "/panel"
	read := func() any { return b.Get(path, master).Value() }
	// A conversation that never saved has the empty panel.
	backendtest.AssertJSON(t, read(), backendtest.Map{"selection": "change", "tabs": []any{}})

	panel := backendtest.Map{
		"selection": "tab-1",
		"tabs": []any{
			backendtest.Map{"id": "tab-1", "kind": "browser", "data": backendtest.Map{"url": "about:blank"}},
			backendtest.Map{"id": "tab-2", "kind": "a kind the backend never heard of", "data": []any{1, backendtest.Map{"nested": true}}},
		},
	}
	b.Put(path, master, panel).Expect(http.StatusNoContent)
	backendtest.AssertJSON(t, read(), panel)
	// The last save wins.
	emptied := backendtest.Map{"selection": "file", "tabs": []any{}}
	b.Put(path, master, emptied).Expect(http.StatusNoContent)
	backendtest.AssertJSON(t, read(), emptied)

	tabs := func(count int) backendtest.Map {
		list := make([]any, count)
		for index := range list {
			list[index] = backendtest.Map{"id": fmt.Sprintf("t%d", index), "kind": "page", "data": nil}
		}
		return backendtest.Map{"selection": "change", "tabs": list}
	}
	for _, refusal := range []struct {
		body   backendtest.Map
		status int
		code   string
	}{
		{backendtest.Map{"selection": "change"}, http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"selection": "change", "tabs": []any{
			backendtest.Map{"id": "a", "kind": "page", "data": nil, "status": "loading"},
		}}, http.StatusBadRequest, "invalid_body"},
		{tabs(65), http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"selection": "change", "tabs": []any{
			backendtest.Map{"id": "a", "kind": "page", "data": strings.Repeat("x", 70_000)},
		}}, http.StatusRequestEntityTooLarge, "too_large"},
	} {
		if status, code := b.Put(path, master, refusal.body).Refusal(); status != refusal.status || code != refusal.code {
			t.Fatalf("%v is %d %s", refusal.body, status, code)
		}
	}
	b.Put(path, master, tabs(64)).Expect(http.StatusNoContent)
	backendtest.AssertJSON(t, read(), tabs(64))

	unknown := b.Get("/api/conversations/"+newID()+"/panel", master)
	if status, code := unknown.Refusal(); status != http.StatusNotFound || code != "conversation_not_found" {
		t.Fatalf("an unknown conversation's panel is %d %s", status, code)
	}
	// An archived conversation reads its panel and refuses a save.
	b.Patch("/api/conversations/"+id, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, read(), tabs(64))
	if status, code := b.Put(path, master, panel).Refusal(); status != http.StatusConflict || code != "conversation_archived" {
		t.Fatalf("a save to an archived conversation is %d %s", status, code)
	}
	b.Stop()
}

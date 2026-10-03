package backend_test

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// hostScenario drives the host scenarios through their HTTP boundary.
type hostScenario struct {
	t       *testing.T
	ctx     context.Context
	h       *backendtest.Harness
	b       *backendtest.TestBackend
	user    backendtest.Session
	manager *backendtest.ScriptedManager
}

func newHostScenario(t *testing.T, program string) *hostScenario {
	t.Helper()
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	if program != "" {
		built, err := backendtest.BuildPackage(t.Context(), t, program)
		if err != nil {
			t.Fatal(err)
		}
		if err := h.UsePackage(t.Context(), built); err != nil {
			t.Fatal(err)
		}
	}
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	return &hostScenario{t, t.Context(), h, b, user, manager}
}
func (s *hostScenario) pair(name string) *backendtest.Paired {
	s.t.Helper()
	p, err := s.b.Pair(s.ctx, s.t, &s.user, name)
	if err != nil {
		s.t.Fatal(err)
	}
	return p
}
func (s *hostScenario) workspace(device *backendtest.Paired, path, name string) webapi.WorkspaceDTO {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/workspaces", fmt.Sprintf(`{"kind":"device","deviceId":%q,"path":%q,"name":%q}`, device.ID(), path, name), 201)
	w, err := webapi.DecodeWorkspaceAnswer(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return w.Workspace
}
func (s *hostScenario) workspaceNames(want ...string) {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/workspaces", "", 200)
	list, err := webapi.DecodeWorkspaces(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	names := make([]string, len(list.Workspaces))
	for i, w := range list.Workspaces {
		names[i] = w.Name
	}
	if diff := cmp.Diff(want, names); diff != "" {
		s.t.Fatal(diff)
	}
}
func (s *hostScenario) conversations(query string) []webapi.ConversationSummary {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations"+query, "", 200)
	list, err := webapi.DecodeConversations(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return list.Conversations
}
func (s *hostScenario) order(query string, want ...string) {
	s.t.Helper()
	list := s.conversations(query)
	ids := make([]string, len(list))
	for i, c := range list {
		ids[i] = string(c.ID)
	}
	if diff := cmp.Diff(want, ids); diff != "" {
		s.t.Fatal(diff)
	}
}
func (s *hostScenario) reorder(kind, id string, before *string, status int) {
	s.t.Helper()
	data, err := contract.EncodeObject([]contract.Field{{Name: "kind", Value: kind}, {Name: "id", Value: id}, {Name: "beforeId", Value: before}})
	if err != nil {
		s.t.Fatal(err)
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/sidebar/reorder", string(data), status)
}

// A real paired runner establishes device identity; file and sidebar observations stay on the API.
func TestWorkspaceKeepsDeviceDirectoryWhileTargeted(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	laptop := s.pair("laptop")
	home := laptop.Runner.Home()
	notes := s.workspace(laptop, home, "  notes  ")
	s.workspace(laptop, filepath.Join(home, "site"), "site")
	s.workspaceNames("notes", "site")
	channel, err := s.b.Sync(s.ctx, t, &s.user)
	if err != nil {
		t.Fatal(err)
	}
	state, err := channel.Snapshot(s.ctx)
	if err != nil {
		t.Fatal(err)
	}
	if state.Workspaces[0].ID != notes.ID || state.Workspaces[0].Path != home {
		t.Fatalf("snapshot: %+v", state.Workspaces)
	}
	for _, tc := range []struct {
		body   string
		status int
		code   webapi.ErrorCode
	}{
		{fmt.Sprintf(`{"kind":"device","deviceId":"nothing","path":%q,"name":"x"}`, home), 404, webapi.ErrorCodeDeviceNotFound},
		{fmt.Sprintf(`{"kind":"device","deviceId":%q,"path":"relative","name":"x"}`, laptop.ID()), 400, webapi.ErrorCodeInvalidBody},
		{fmt.Sprintf(`{"kind":"device","deviceId":%q,"path":%q,"name":"   "}`, laptop.ID(), home), 400, webapi.ErrorCodeInvalidBody},
		{fmt.Sprintf(`{"deviceId":%q,"path":%q,"name":"untagged"}`, laptop.ID(), home), 400, webapi.ErrorCodeInvalidBody},
	} {
		conversationRefusal(t, conversationRequest(s.ctx, t, s.b, &s.user, "POST", "/api/workspaces", tc.body, tc.status), tc.code)
	}
	route := "/api/workspaces/" + string(notes.ID)
	renamed := conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route, `{"name":"journal"}`, 200)
	w, err := webapi.DecodeWorkspaceAnswer(renamed.Body)
	if err != nil {
		t.Fatal(err)
	}
	if w.Workspace.Name != "journal" {
		t.Fatal(w.Workspace.Name)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/workspaces/nothing", `{"name":"x"}`, 404), webapi.ErrorCodeWorkspaceNotFound)
	const id = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	conversationCreate(s.ctx, s.t, s.b, &s.user, id)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+id, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, notes.ID), 200)
	if got := s.conversations("")[0].Cwd; got != home {
		t.Fatalf("cwd %s", got)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route, "", 409), webapi.ErrorCodeWorkspaceInUse)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+string(laptop.ID()), "", 409), webapi.ErrorCodeDeviceInUse)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+id, `{"target":{"kind":"cloud"}}`, 200)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route, "", 204)
	s.workspaceNames("site")
	info, err := os.Stat(home)
	if err != nil || !info.IsDir() {
		t.Fatalf("home: %v", err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route, "", 404), webapi.ErrorCodeWorkspaceNotFound)
}

func TestSidebarOrderSurvivesPatchesAndRestart(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	a, b, c := "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a03"
	for _, id := range []string{a, b, c} {
		conversationCreate(s.ctx, s.t, s.b, &s.user, id)
	}
	s.order("", c, b, a)
	s.reorder("conversation", a, &c, 204)
	s.order("", a, c, b)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+a, `{"title":"kept title"}`, 200)
	s.reorder("conversation", c, nil, 204)
	s.order("", a, b, c)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+b, `{"pinned":true}`, 200)
	s.order("", b, a, c)
	s.reorder("conversation", a, &b, 409)
	laptop := s.pair("laptop")
	home := laptop.Runner.Home()
	notes := s.workspace(laptop, home, "notes")
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+c, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, notes.ID), 200)
	s.reorder("conversation", a, &c, 409)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+a, `{"archived":true}`, 200)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/sidebar/reorder", fmt.Sprintf(`{"kind":"conversation","id":%q,"beforeId":null}`, a), 409), webapi.ErrorCodeInvalidOrder)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/sidebar/reorder", fmt.Sprintf(`{"kind":"project","id":%q,"beforeId":null}`, a), 400), webapi.ErrorCodeInvalidBody)
	site := s.workspace(laptop, filepath.Join(home, "site"), "site")
	s.workspaceNames("notes", "site")
	before := string(notes.ID)
	s.reorder("workspace", string(site.ID), &before, http.StatusNoContent)
	s.workspaceNames("site", "notes")
	if err := laptop.Runner.Stop(s.ctx); err != nil {
		t.Fatal(err)
	}
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	s.b, err = s.h.Start(s.ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	s.order("?archived=false", b, c)
	s.workspaceNames("site", "notes")
}

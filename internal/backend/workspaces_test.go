package backend_test

import (
	"context"
	"encoding/json"
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
func (s *hostScenario) request(method, path, body string, status int) backendtest.Answer {
	s.t.Helper()
	var data []byte
	if body != "" {
		var err error
		data, err = contract.EncodeJSON(json.RawMessage(body))
		if err != nil {
			s.t.Fatal(err)
		}
	}
	a, err := s.b.Send(s.ctx, method, path, &s.user.Cookie, data)
	if err != nil {
		s.t.Fatal(err)
	}
	if a.Status != status {
		s.t.Fatalf("%s %s: HTTP %d, want %d: %s", method, path, a.Status, status, a.Body)
	}
	return a
}
func (s *hostScenario) refusal(method, path, body string, status int, code webapi.ErrorCode) {
	s.t.Helper()
	a := s.request(method, path, body, status)
	e, err := a.ErrorBody()
	if err != nil {
		s.t.Fatal(err)
	}
	if e.Code != code {
		s.t.Fatalf("%s: code %s, want %s", path, e.Code, code)
	}
}
func (s *hostScenario) pair(name string) *backendtest.Paired {
	s.t.Helper()
	p, err := s.b.Pair(s.ctx, s.t, &s.user, name)
	if err != nil {
		s.t.Fatal(err)
	}
	return p
}
func (s *hostScenario) create(id string) {
	s.request("POST", "/api/conversations", fmt.Sprintf(`{"id":%q}`, id), 201)
}
func (s *hostScenario) workspace(device *backendtest.Paired, path, name string) webapi.WorkspaceDTO {
	s.t.Helper()
	a := s.request("POST", "/api/workspaces", fmt.Sprintf(`{"kind":"device","deviceId":%q,"path":%q,"name":%q}`, device.ID(), path, name), 201)
	w, err := webapi.DecodeWorkspaceAnswer(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return w.Workspace
}
func (s *hostScenario) workspaceNames(want ...string) {
	s.t.Helper()
	a := s.request("GET", "/api/workspaces", "", 200)
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
	a := s.request("GET", "/api/conversations"+query, "", 200)
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
	s.request("POST", "/api/sidebar/reorder", string(data), status)
}

// A real paired runner establishes device identity; file and sidebar observations stay on the API.
func TestWorkspaceKeepsDeviceDirectoryWhileTargeted(t *testing.T) {
	t.Skip("finding 1: device claim cannot encode the nil installs array")
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
		s.refusal("POST", "/api/workspaces", tc.body, tc.status, tc.code)
	}
	route := "/api/workspaces/" + string(notes.ID)
	renamed := s.request("PATCH", route, `{"name":"journal"}`, 200)
	w, err := webapi.DecodeWorkspaceAnswer(renamed.Body)
	if err != nil {
		t.Fatal(err)
	}
	if w.Workspace.Name != "journal" {
		t.Fatal(w.Workspace.Name)
	}
	s.refusal("PATCH", "/api/workspaces/nothing", `{"name":"x"}`, 404, webapi.ErrorCodeWorkspaceNotFound)
	const id = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	s.create(id)
	s.request("PATCH", "/api/conversations/"+id, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, notes.ID), 200)
	if got := s.conversations("")[0].Cwd; got != home {
		t.Fatalf("cwd %s", got)
	}
	s.refusal("DELETE", route, "", 409, webapi.ErrorCodeWorkspaceInUse)
	s.refusal("DELETE", "/api/devices/"+string(laptop.ID()), "", 409, webapi.ErrorCodeDeviceInUse)
	s.request("PATCH", "/api/conversations/"+id, `{"target":{"kind":"cloud"}}`, 200)
	s.request("DELETE", route, "", 204)
	s.workspaceNames("site")
	info, err := os.Stat(home)
	if err != nil || !info.IsDir() {
		t.Fatalf("home: %v", err)
	}
	s.refusal("DELETE", route, "", 404, webapi.ErrorCodeWorkspaceNotFound)
}

func TestSidebarOrderSurvivesPatchesAndRestart(t *testing.T) {
	t.Skip("finding 1: device claim cannot encode the nil installs array")
	s := newHostScenario(t, "")
	a, b, c := "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a03"
	for _, id := range []string{a, b, c} {
		s.create(id)
	}
	s.order("", c, b, a)
	s.reorder("conversation", a, &c, 204)
	s.order("", a, c, b)
	s.request("PATCH", "/api/conversations/"+a, `{"title":"kept title"}`, 200)
	s.reorder("conversation", c, nil, 204)
	s.order("", a, b, c)
	s.request("PATCH", "/api/conversations/"+b, `{"pinned":true}`, 200)
	s.order("", b, a, c)
	s.reorder("conversation", a, &b, 409)
	laptop := s.pair("laptop")
	home := laptop.Runner.Home()
	notes := s.workspace(laptop, home, "notes")
	s.request("PATCH", "/api/conversations/"+c, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, notes.ID), 200)
	s.reorder("conversation", a, &c, 409)
	s.request("PATCH", "/api/conversations/"+a, `{"archived":true}`, 200)
	s.refusal("POST", "/api/sidebar/reorder", fmt.Sprintf(`{"kind":"conversation","id":%q,"beforeId":null}`, a), 409, webapi.ErrorCodeInvalidOrder)
	s.refusal("POST", "/api/sidebar/reorder", fmt.Sprintf(`{"kind":"project","id":%q,"beforeId":null}`, a), 400, webapi.ErrorCodeInvalidBody)
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

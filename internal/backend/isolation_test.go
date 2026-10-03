package backend_test

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/url"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/webapi"
)

// isolationUpload stores bytes through the attachment boundary and returns its metadata.
func isolationUpload(s *hostScenario, media string, data []byte) webapi.AttachmentDTO {
	s.t.Helper()
	response, err := s.b.Response(
		s.ctx,
		"POST",
		"/api/attachments?name=file",
		&s.user,
		http.Header{"Content-Type": {media}},
		bytes.NewReader(data),
	)
	if err != nil {
		s.t.Fatal(err)
	}
	answer, err := backendtest.ReadAnswer(s.ctx, response)
	if err != nil {
		s.t.Fatal(err)
	}
	if answer.Status != 201 {
		s.t.Fatalf("upload: %d %s", answer.Status, answer.Body)
	}
	decoded, err := webapi.DecodeAttachmentAnswer(answer.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return decoded.Attachment
}

// isolationLists observes only the lists visible to the current actor.
func isolationLists(s *hostScenario) [3]int {
	s.t.Helper()
	conversations := s.conversations("")
	devices, err := s.b.Devices(s.ctx, &s.user)
	if err != nil {
		s.t.Fatal(err)
	}
	answer := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/workspaces", "", 200)
	workspaces, err := webapi.DecodeWorkspaces(answer.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return [3]int{len(conversations), len(devices), len(workspaces.Workspaces)}
}

// TestIsolationHidesEveryOtherUsersObjectAndRevokedDevice checks user isolation and device identity after revocation.
// Real runners make revocation and a new owner's pairing observable at the API.
func TestIsolationHidesEveryOtherUsersObjectAndRevokedDevice(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	master := s.user
	users := make([]backendtest.Session, 0, 2)
	for _, name := range []string{"alice", "bob"} {
		email, password := name+"@example.test", name+"-pass-1"
		conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"POST",
			"/api/users",
			fmt.Sprintf(`{"email":%q,"password":%q,"role":"user"}`, email, password),
			201,
		)
		user, err := s.b.Login(s.ctx, email, password)
		if err != nil {
			t.Fatal(err)
		}
		users = append(users, user)
	}
	alice, bob := users[0], users[1]
	s.user = alice
	laptop := s.pair("alice-laptop")
	device, home := string(laptop.ID()), laptop.Runner.Home()
	workspace := s.workspace(laptop, home, "proj")
	aliceConversation, bobConversation := hostsConversation, "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a02"
	conversationCreate(s.ctx, s.t, s.b, &s.user, aliceConversation)
	base := "/api/conversations/" + aliceConversation
	conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", base+"/hosts", fmt.Sprintf(`{"deviceId":%q}`, device), 201)
	png := []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0xff, 0xfe, 1}
	image := isolationUpload(s, "image/png", png)
	private := []byte("alice private file")
	fileUpload := isolationUpload(s, "text/plain", private)
	if string(fileUpload.Sha256) != fmt.Sprintf("%x", sha256.Sum256(private)) {
		t.Fatal(fileUpload.Sha256)
	}
	socket, err := s.b.Conversation(s.ctx, t, &alice, aliceConversation)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	s.user = bob
	conversationCreate(s.ctx, s.t, s.b, &s.user, bobConversation)
	made, file := home+"/made", url.QueryEscape(home)
	other := "/api/conversations/" + bobConversation
	denied := []struct{ method, path, body string }{
		{"GET", "/api/blobs/" + string(image.Sha256), ""},
		{"GET", "/api/blobs/" + string(fileUpload.Sha256), ""},
		{"GET", base + "/transcript", ""},
		{"PATCH", base, `{"title":"taken"}`},
		{"PATCH", base, `{"archived":true}`},
		{"GET", base + "/hosts", ""},
		{"GET", base + "/fs", ""},
		{"POST", base + "/fs", fmt.Sprintf(`{"path":%q}`, made)},
		{"GET", base + "/fs/file?path=" + file, ""},
		{"GET", base + "/fs/raw?path=" + file, ""},
		{"GET", base + "/changes", ""},
		{"GET", base + "/panel", ""},
		{"PUT", base + "/panel", `{"selection":"change","tabs":[]}`},
		{"GET", base + "/draft", ""},
		{"PUT", base + "/draft", `{"base":0,"text":"","files":[]}`},
		{"POST", base + "/draft/replaced", `{"action":"dismiss","revision":1}`},
		{
			"PUT",
			other + "/draft",
			fmt.Sprintf(`{"base":0,"text":"\uFFFC","files":[{"type":"upload","ref":%q,"fileName":"a.png"}]}`, image.ID),
		},
		{"POST", base + "/plugins/browser/calls/tabs", `{}`},
		{"POST", base + "/activity", ""},
		{"GET", base + "/hosts/" + device + "/fs", ""},
		{"POST", base + "/hosts/" + device + "/fs", fmt.Sprintf(`{"path":%q}`, made)},
		{"GET", other + "/hosts/" + device + "/fs", ""},
		{"POST", base + "/hosts", fmt.Sprintf(`{"deviceId":%q}`, device)},
		{"PATCH", base + "/hosts/" + device, `{"name":"taken"}`},
		{"DELETE", base + "/hosts/" + device, ""},
		{"PATCH", other, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, workspace.ID)},
		{"POST", other + "/hosts", fmt.Sprintf(`{"deviceId":%q}`, device)},
		{"DELETE", "/api/devices/" + device, ""},
		{"GET", "/api/devices/" + device + "/fs?path=" + file, ""},
		{"POST", "/api/devices/" + device + "/fs", fmt.Sprintf(`{"path":%q}`, made)},
		{"PATCH", "/api/workspaces/" + string(workspace.ID), `{"name":"taken"}`},
		{"DELETE", "/api/workspaces/" + string(workspace.ID), ""},
		{
			"POST",
			"/api/workspaces",
			fmt.Sprintf(`{"kind":"device","deviceId":%q,"path":%q,"name":"squat"}`, device, home),
		},
	}
	for i, actor := range []backendtest.Session{bob, master} {
		s.user = actor
		for _, route := range denied {
			conversationRequest(s.ctx, s.t, s.b, &s.user, route.method, route.path, route.body, 404)
		}
		_, refused, err := s.b.ConversationFrom(s.ctx, t, &actor, aliceConversation, s.b.URL)
		if err == nil || refused.Status != 404 {
			t.Fatalf("foreign socket: %v %v", refused, err)
		}
		want := [3]int{1 - i, 0, 0}
		if got := isolationLists(s); got != want {
			t.Fatalf("lists: %v want %v", got, want)
		}
	}
	s.user = alice
	if len(s.hosts()) != 1 || isolationLists(s) != [3]int{1, 1, 1} {
		t.Fatal("Alice's objects changed")
	}
	for _, blob := range []struct {
		hash string
		data []byte
	}{{string(image.Sha256), png}, {string(fileUpload.Sha256), private}} {
		if got := conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"GET",
			"/api/blobs/"+blob.hash,
			"",
			200,
		).Body; !bytes.Equal(
			got,
			blob.data,
		) {
			t.Fatal("blob changed")
		}
	}
	s.user = bob
	if isolationUpload(s, "image/png", png).Sha256 != image.Sha256 {
		t.Fatal("same bytes changed hash")
	}
	if !bytes.Equal(
		conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/blobs/"+string(image.Sha256), "", 200).Body,
		png,
	) {
		t.Fatal("Bob's image differs")
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/blobs/"+string(fileUpload.Sha256), "", 404)
	s.user = alice
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+device, "", 409)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/workspaces/"+string(workspace.ID), "", 204)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+device, "", 204)
	if len(s.hosts()) != 0 {
		t.Fatal("revoked attachment remains")
	}
	// The runner must end on its own; its exit status does not matter.
	if err := laptop.Runner.Exited(s.ctx); err != nil && s.ctx.Err() != nil {
		t.Fatal(err)
	}
	s.user = bob
	again := s.pair("alice-laptop")
	if again.ID() == laptop.ID() {
		t.Fatal("revoked identity reused")
	}
	devices, err := s.b.Devices(s.ctx, &bob)
	if err != nil || len(devices) != 1 || devices[0].ID != again.ID() {
		t.Fatalf("devices: %+v %v", devices, err)
	}
	s.user = alice
	if got := isolationLists(s); got != [3]int{1, 0, 0} {
		t.Fatal(got)
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", "/api/devices/"+string(again.ID()), "", 404)
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
}

package backendtest_test

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/url"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// pngHead is a PNG image's first bytes, from which the backend reads its type.
var pngHead = []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01}

// createdUser creates a user as the master and signs them in.
func createdUser(t *testing.T, b *backendtest.Backend, master *backendtest.Session, email string) *backendtest.Session {
	t.Helper()
	name, _, _ := strings.Cut(email, "@")
	return b.CreateUser(master, email, name+"-pass-1", "user")
}

// listLengths is how many entries each list holds for the session: the
// conversations, the devices and the workspaces.
func listLengths(b *backendtest.Backend, session *backendtest.Session) []int {
	var lengths []int
	for _, list := range []struct{ path, field string }{
		{"/api/conversations", "conversations"},
		{"/api/devices", "devices"},
		{"/api/workspaces", "workspaces"},
	} {
		entries, _ := b.Get(list.path, session).At(list.field).([]any)
		lengths = append(lengths, len(entries))
	}
	return lengths
}

// Cost: one backend and a real runner, about a second, and about sixty requests
// for each of two actors.
func TestAnotherUsersObjectsAnswer404OnEveryRouteToUsersAndAdminsAlikeAndARevokedDeviceIsGone(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	alice := createdUser(t, b, master, "alice@example.test")
	bob := createdUser(t, b, master, "bob@example.test")

	// Alice's world: a device, a workspace on it, a conversation with the device
	// attached, and two uploads.
	laptop := b.Pair(alice, "alice-laptop")
	device := laptop.ID()
	home := laptop.Home()
	workspace := createWorkspace(t, b, alice, backendtest.Map{"kind": "device", "deviceId": device, "path": home, "name": "proj"})
	conversation := createConversation(t, b, alice)
	b.Post("/api/conversations/"+conversation+"/hosts", alice, backendtest.Map{"deviceId": device}).Expect(http.StatusCreated)
	imageUpload := upload(t, b, alice, "file", "image/png", pngHead)
	image := imageUpload["sha256"].(string)
	private := []byte("alice private file")
	privateUpload := upload(t, b, alice, "file", "text/plain", private)
	privateHash := privateUpload["sha256"].(string)
	if sum := sha256.Sum256(private); privateHash != hex.EncodeToString(sum[:]) {
		t.Fatalf("the upload's hash is %s", privateHash)
	}
	b.Connect(alice, conversation).Close()
	bobs := createConversation(t, b, bob)

	c := conversation
	made := home + "/made"
	file := url.QueryEscape(home)
	type route struct {
		method, path string
		body         any
	}
	denied := []route{
		{http.MethodGet, "/api/blobs/" + image, nil},
		{http.MethodGet, "/api/blobs/" + privateHash, nil},
		{http.MethodGet, "/api/conversations/" + c + "/transcript", nil},
		{http.MethodPatch, "/api/conversations/" + c, backendtest.Map{"title": "taken"}},
		{http.MethodPatch, "/api/conversations/" + c, backendtest.Map{"archived": true}},
		{http.MethodGet, "/api/conversations/" + c + "/hosts", nil},
		{http.MethodGet, "/api/conversations/" + c + "/fs", nil},
		{http.MethodPost, "/api/conversations/" + c + "/fs", backendtest.Map{"path": made}},
		{http.MethodGet, "/api/conversations/" + c + "/fs/file?path=" + file, nil},
		{http.MethodGet, "/api/conversations/" + c + "/fs/raw?path=" + file, nil},
		{http.MethodGet, "/api/conversations/" + c + "/changes", nil},
		{http.MethodGet, "/api/conversations/" + c + "/panel", nil},
		{http.MethodPut, "/api/conversations/" + c + "/panel", backendtest.Map{"selection": "change", "tabs": []any{}}},
		{http.MethodGet, "/api/conversations/" + c + "/draft", nil},
		{http.MethodPut, "/api/conversations/" + c + "/draft", backendtest.Map{"base": 0, "text": "", "files": []any{}}},
		{http.MethodPost, "/api/conversations/" + c + "/draft/replaced", backendtest.Map{"action": "dismiss", "revision": 1}},
		{http.MethodPut, "/api/conversations/" + bobs + "/draft", backendtest.Map{
			"base": 0, "text": draftMark, "files": []any{backendtest.Map{"type": "upload", "ref": imageUpload["id"], "fileName": "a.png"}},
		}},
		{http.MethodGet, "/api/conversations/" + c + "/browser/tabs", nil},
		{http.MethodPost, "/api/conversations/" + c + "/activity", nil},
		{http.MethodGet, "/api/conversations/" + c + "/hosts/" + device + "/fs", nil},
		{http.MethodPost, "/api/conversations/" + c + "/hosts/" + device + "/fs", backendtest.Map{"path": made}},
		{http.MethodGet, "/api/conversations/" + bobs + "/hosts/" + device + "/fs", nil},
		{http.MethodPost, "/api/conversations/" + c + "/hosts", backendtest.Map{"deviceId": device}},
		{http.MethodPatch, "/api/conversations/" + c + "/hosts/" + device, backendtest.Map{"name": "taken"}},
		{http.MethodDelete, "/api/conversations/" + c + "/hosts/" + device, nil},
		{http.MethodPatch, "/api/conversations/" + bobs, backendtest.Map{"target": backendtest.Map{"kind": "workspace", "workspaceId": workspace}}},
		{http.MethodPost, "/api/conversations/" + bobs + "/hosts", backendtest.Map{"deviceId": device}},
		{http.MethodDelete, "/api/devices/" + device, nil},
		{http.MethodGet, "/api/devices/" + device + "/fs?path=" + file, nil},
		{http.MethodPost, "/api/devices/" + device + "/fs", backendtest.Map{"path": made}},
		{http.MethodPatch, "/api/workspaces/" + workspace, backendtest.Map{"name": "taken"}},
		{http.MethodDelete, "/api/workspaces/" + workspace, nil},
		{http.MethodPost, "/api/workspaces", backendtest.Map{"kind": "device", "deviceId": device, "path": home, "name": "squat"}},
	}
	for _, actor := range []struct {
		session *backendtest.Session
		lists   []int
	}{{bob, []int{1, 0, 0}}, {master, []int{0, 0, 0}}} {
		for _, denial := range denied {
			answered := b.Do(backendtest.Request{Method: denial.method, Path: denial.path, Session: actor.session, Body: denial.body})
			if answered.Status != http.StatusNotFound {
				t.Fatalf("%s %s %s is %d: %s", actor.session.User.Email, denial.method, denial.path, answered.Status, answered.Body)
			}
		}
		if _, status, _ := b.TryConnect(actor.session, conversation); status != http.StatusNotFound {
			t.Fatalf("%s opens the socket: %d", actor.session.User.Email, status)
		}
		if lists := listLengths(b, actor.session); !slices.Equal(lists, actor.lists) {
			t.Fatalf("%s lists %v, not %v", actor.session.User.Email, lists, actor.lists)
		}
	}

	// Alice still has everything.
	hosts, _ := b.Get("/api/conversations/"+c+"/hosts", alice).At("hosts").([]any)
	if len(hosts) != 1 {
		t.Fatalf("alice's conversation has %d hosts", len(hosts))
	}
	if lists := listLengths(b, alice); !slices.Equal(lists, []int{1, 1, 1}) {
		t.Fatalf("alice lists %v", lists)
	}
	if got := b.Get("/api/blobs/"+image, alice).Body; string(got) != string(pngHead) {
		t.Fatalf("alice reads her image as %v", got)
	}
	if got := b.Get("/api/blobs/"+privateHash, alice).Body; string(got) != string(private) {
		t.Fatalf("alice reads her file as %q", got)
	}
	// The same bytes are Bob's only once he stores his own copy.
	if hash := upload(t, b, bob, "file", "image/png", pngHead)["sha256"]; hash != image {
		t.Fatalf("bob's copy is named %v", hash)
	}
	if got := b.Get("/api/blobs/"+image, bob).Body; string(got) != string(pngHead) {
		t.Fatalf("bob reads his image as %v", got)
	}
	b.Get("/api/blobs/"+privateHash, bob).Expect(http.StatusNotFound)

	// A revoke waits for the workspace to go, then the runner is refused for good
	// and the attachment goes with the device.
	b.Delete("/api/devices/"+device, alice).Expect(http.StatusConflict)
	b.Delete("/api/workspaces/"+workspace, alice).Expect(http.StatusNoContent)
	b.Delete("/api/devices/"+device, alice).Expect(http.StatusNoContent)
	backendtest.AssertJSON(t, b.Get("/api/conversations/"+c+"/hosts", alice).At("hosts"), []any{})
	backendtest.Eventually(t, "the revoked runner stops", func() bool { return !laptop.Runner.Running() })
	// Pairing the machine again is a new device, here Bob's.
	again := b.Pair(bob, "alice-laptop")
	if again.ID() == device {
		t.Fatal("pairing again gives the device back")
	}
	var ids []string
	devices, _ := b.Get("/api/devices", bob).At("devices").([]any)
	for _, entry := range devices {
		ids = append(ids, backendtest.At(entry, "id").(string))
	}
	if !slices.Equal(ids, []string{again.ID()}) {
		t.Fatalf("bob's devices are %v", ids)
	}
	if lists := listLengths(b, alice); !slices.Equal(lists, []int{1, 0, 0}) {
		t.Fatalf("alice lists %v", lists)
	}
	b.Delete("/api/devices/"+again.ID(), alice).Expect(http.StatusNotFound)
	b.Stop()
}

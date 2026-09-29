package backendtest_test

import (
	"io"
	"net/http"
	"slices"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

const hostsConversation = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01"

// cloudConversation starts a backend with the master signed in and a
// conversation on the Cloud.
func cloudConversation(t *testing.T, h *backendtest.Harness) (*backendtest.Backend, *backendtest.Session) {
	t.Helper()
	b, master := h.StartSetUp()
	b.CreateConversation(master, hostsConversation)
	return b, master
}

func deviceTarget(device *backendtest.Paired, path string) backendtest.Map {
	return backendtest.Map{"kind": "device", "deviceId": device.ID(), "path": path}
}

func switchTarget(b *backendtest.Backend, session *backendtest.Session, target backendtest.Map) *backendtest.Answer {
	return b.Patch("/api/conversations/"+hostsConversation, session, backendtest.Map{"target": target})
}

// attachedHosts lists the conversation's attached hosts.
func attachedHosts(b *backendtest.Backend, session *backendtest.Session) []any {
	hosts, _ := b.Get("/api/conversations/"+hostsConversation+"/hosts", session).Expect(http.StatusOK).At("hosts").([]any)
	return hosts
}

// hostsSummary is the conversation as the list shows it.
func hostsSummary(b *backendtest.Backend, session *backendtest.Session) map[string]any {
	listed, _ := b.Get("/api/conversations?archived=false", session).At("conversations").([]any)
	for _, conversation := range listed {
		if backendtest.At(conversation, "id") == hostsConversation {
			return conversation.(map[string]any)
		}
	}
	return nil
}

// Cost: one backend and three real runners, about a second.
func TestASwitchMovesTheWorkAndAttachesTheDeviceItLeaves(t *testing.T) {
	t.Parallel()
	b, master := cloudConversation(t, backendtest.New(t))
	laptop := b.Pair(master, "laptop")
	ci := b.Pair(master, "ci")
	onLaptop := directoryIn(t, laptop, "work")
	onCI := directoryIn(t, ci, "build")

	moved := switchTarget(b, master, deviceTarget(laptop, onLaptop)).Expect(http.StatusOK)
	backendtest.AssertJSON(t, moved.At("results"), []any{backendtest.Map{"field": "target", "status": "applied"}})
	before := hostsSummary(b, master)
	if before["cwd"] != onLaptop {
		t.Fatalf("the conversation works in %v", before["cwd"])
	}
	// The Cloud was never used, so no device is left behind.
	if hosts := attachedHosts(b, master); len(hosts) != 0 {
		t.Fatalf("hosts are attached: %v", hosts)
	}

	// The files follow the target; what stayed on the laptop stays there.
	writeFile(t, onLaptop+"/report.txt", "on the laptop")
	switchTarget(b, master, deviceTarget(ci, onCI)).Expect(http.StatusOK)
	after := hostsSummary(b, master)
	if after["cwd"] != onCI || after["contextVersion"].(float64) <= before["contextVersion"].(float64) {
		t.Fatalf("the conversation after the switch is %v", after)
	}
	if path := b.Get("/api/conversations/"+hostsConversation+"/fs", master).Str("path"); path != onCI {
		t.Fatalf("the listing starts in %s", path)
	}
	attached := attachedHosts(b, master)
	if len(attached) != 1 || backendtest.At(attached[0], "deviceId") != laptop.ID() || backendtest.At(attached[0], "name") != "laptop" ||
		backendtest.At(attached[0], "cwd") != onLaptop || backendtest.At(attached[0], "online") != true {
		t.Fatalf("the attached hosts are %v", attached)
	}
	left := b.Get("/api/conversations/"+hostsConversation+"/hosts/"+laptop.ID()+"/fs", master)
	var names []string
	entries, _ := left.At("entries").([]any)
	for _, entry := range entries {
		names = append(names, backendtest.At(entry, "name").(string))
	}
	if !slices.Equal(names, []string{"report.txt"}) {
		t.Fatalf("the attached laptop lists %v", names)
	}

	// Back to the laptop: it is main alone, and the device left is attached.
	switchTarget(b, master, deviceTarget(laptop, onLaptop)).Expect(http.StatusOK)
	var devices []any
	for _, host := range attachedHosts(b, master) {
		devices = append(devices, backendtest.At(host, "deviceId"))
	}
	if len(devices) != 1 || devices[0] != ci.ID() {
		t.Fatalf("the attached devices are %v", devices)
	}
	// The conversation's own target is no change.
	unchanged := hostsSummary(b, master)
	switchTarget(b, master, deviceTarget(laptop, onLaptop)).Expect(http.StatusOK)
	if got := hostsSummary(b, master)["contextVersion"]; got != unchanged["contextVersion"] {
		t.Fatalf("the context version moved from %v to %v", unchanged["contextVersion"], got)
	}

	// What the destination names must be the user's.
	wantRefusal(t, switchTarget(b, master, backendtest.Map{"kind": "workspace", "workspaceId": "nowhere"}), http.StatusNotFound, "workspace_not_found", "a missing workspace")
	wantRefusal(t, switchTarget(b, master, backendtest.Map{"kind": "device", "deviceId": "nothing", "path": "/"}), http.StatusNotFound, "device_not_found", "a missing device")
	other := b.CreateUser(master, "user@example.test", "user-pass-1", "user")
	theirs := b.Pair(other, "theirs")
	wantRefusal(t, switchTarget(b, master, deviceTarget(theirs, directoryIn(t, theirs, "x"))), http.StatusNotFound, "device_not_found", "another user's device")
	// An archived conversation takes no switch.
	b.Patch("/api/conversations/"+hostsConversation, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	wantRefusal(t, switchTarget(b, master, deviceTarget(ci, onCI)), http.StatusConflict, "conversation_archived", "a switch of an archived conversation")
	b.Stop()
}

// Cost: one backend and a real runner, about a second, and 64 MiB written.
func TestASwitchEndsTheOpenDownloadInsteadOfWaitingForIt(t *testing.T) {
	t.Parallel()
	b, master := cloudConversation(t, backendtest.New(t))
	laptop := b.Pair(master, "laptop")
	onLaptop := directoryIn(t, laptop, "work")
	switchTarget(b, master, deviceTarget(laptop, onLaptop)).Expect(http.StatusOK)
	writeFile(t, onLaptop+"/long.mp4", string(backendtest.Pattern(64*1024*1024, 0)))
	route := "/api/conversations/" + hostsConversation + "/fs/raw?" + query("path", onLaptop+"/long.mp4")
	playing, err := b.Send(backendtest.Request{Path: route, Session: master})
	if err != nil {
		t.Fatal(err)
	}
	defer playing.Body.Close()
	if playing.StatusCode != http.StatusOK {
		t.Fatalf("the download answers %d", playing.StatusCode)
	}
	if n, err := playing.Body.Read(make([]byte, 1)); n == 0 || err != nil {
		t.Fatalf("the download sends nothing: %v", err)
	}

	// The page reads nothing more; the switch ends the download and goes on.
	switchTarget(b, master, backendtest.Map{"kind": "cloud"}).Expect(http.StatusOK)
	if _, err := io.Copy(io.Discard, playing.Body); err == nil {
		t.Fatal("the download completed")
	}
	b.Stop()
}

// Cost: one backend and four real runners, about a second.
func TestAnAttachedHostIsAttachedOnceNamedUniquelyAndDetached(t *testing.T) {
	t.Parallel()
	b, master := cloudConversation(t, backendtest.New(t))
	laptop := b.Pair(master, "laptop")
	ci := b.Pair(master, "ci")
	spare := b.Pair(master, "ci")
	switchTarget(b, master, deviceTarget(laptop, directoryIn(t, laptop, "work"))).Expect(http.StatusOK)
	route := "/api/conversations/" + hostsConversation + "/hosts"
	attach := func(device string) *backendtest.Answer {
		return b.Post(route, master, backendtest.Map{"deviceId": device})
	}

	attached := attach(ci.ID()).Expect(http.StatusCreated)
	first := attached.At("hosts")
	if backendtest.At(first, "0.name") != "ci" || backendtest.At(first, "0.cwd") != nil || backendtest.At(first, "0.online") != true {
		t.Fatalf("the attached host is %v", first)
	}
	context := hostsSummary(b, master)["contextVersion"]
	// Attached already, it stays as it is.
	backendtest.AssertJSON(t, attach(ci.ID()).At("hosts"), first)
	if got := hostsSummary(b, master)["contextVersion"]; got != context {
		t.Fatalf("attaching again moved the context version from %v to %v", context, got)
	}
	wantRefusal(t, attach(laptop.ID()), http.StatusConflict, "host_is_main", "attaching the main host")
	wantRefusal(t, attach("nothing"), http.StatusNotFound, "device_not_found", "attaching a missing device")
	// A second device of the same name gets a free one.
	both, _ := attach(spare.ID()).At("hosts").([]any)
	if len(both) != 2 || backendtest.At(both[0], "name") != "ci" || backendtest.At(both[1], "name") != "ci-2" {
		t.Fatalf("the attached hosts are %v", both)
	}

	rename := func(device, name string) *backendtest.Answer {
		return b.Patch(route+"/"+device, master, backendtest.Map{"name": name})
	}
	renamed := rename(ci.ID(), "  builder ").Expect(http.StatusOK)
	if renamed.At("hosts.0.name") != "builder" {
		t.Fatalf("the renamed hosts are %s", renamed.Body)
	}
	wantRefusal(t, rename(spare.ID(), "builder"), http.StatusConflict, "name_taken", "a name that is taken")
	wantRefusal(t, rename(laptop.ID(), "main"), http.StatusNotFound, "host_not_attached", "renaming the main host")
	wantRefusal(t, rename(ci.ID(), "   "), http.StatusBadRequest, "invalid_body", "a blank name")

	// A detach is a transition; a device that is not attached detaches as
	// nothing.
	b.Delete(route+"/"+ci.ID(), master).Expect(http.StatusNoContent)
	var left []any
	for _, host := range attachedHosts(b, master) {
		left = append(left, backendtest.At(host, "deviceId"))
	}
	if len(left) != 1 || left[0] != spare.ID() {
		t.Fatalf("the attached devices are %v", left)
	}
	b.Delete(route+"/"+ci.ID(), master).Expect(http.StatusNoContent)
	// A detached device is no Host of the conversation.
	wantRefusal(t, b.Get(route+"/"+ci.ID()+"/fs", master), http.StatusNotFound, "host_not_attached", "the files of a detached device")
	b.Stop()
}

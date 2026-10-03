package backend_test

import (
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/webapi"
)

const hostsConversation = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01"

func (s *hostScenario) directory(p *backendtest.Paired, name string) string {
	s.t.Helper()
	path := filepath.Join(p.Runner.Home(), name)
	if err := os.MkdirAll(path, 0755); err != nil {
		s.t.Fatal(err)
	}
	return path
}
func (s *hostScenario) move(p *backendtest.Paired, path string) backendtest.Answer {
	s.t.Helper()
	return s.request("PATCH", "/api/conversations/"+hostsConversation, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, p.ID(), path), 200)
}
func (s *hostScenario) hosts() []webapi.AttachedHost {
	s.t.Helper()
	a := s.request("GET", "/api/conversations/"+hostsConversation+"/hosts", "", 200)
	hosts, err := webapi.DecodeAttachedHosts(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return hosts.Hosts
}
func (s *hostScenario) directoryListing(path string) webapi.Directory {
	s.t.Helper()
	a := s.request("GET", path, "", 200)
	d, err := webapi.DecodeDirectory(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return d
}

// Pairing real runners and reading both devices proves the switch moves access without moving files.
func TestTargetSwitchAttachesPreviousDevice(t *testing.T) {
	s := newHostScenario(t, "")
	s.create(hostsConversation)
	laptop, ci := s.pair("laptop"), s.pair("ci")
	onLaptop, onCI := s.directory(laptop, "work"), s.directory(ci, "build")
	moved := s.move(laptop, onLaptop)
	update, err := webapi.DecodeConversationUpdate(moved.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(update.Results) != 1 {
		t.Fatalf("results: %+v", update.Results)
	}
	applied, ok := update.Results[0].(*webapi.FieldResultApplied)
	if !ok || applied.Field != webapi.PatchField("target") {
		t.Fatalf("target result: %+v", update.Results)
	}
	before := s.conversations("?archived=false")[0]
	if before.Cwd != onLaptop {
		t.Fatal(before.Cwd)
	}
	if len(s.hosts()) != 0 {
		t.Fatal("unused Cloud attached")
	}
	if err := os.WriteFile(filepath.Join(onLaptop, "report.txt"), []byte("on the laptop"), 0644); err != nil {
		t.Fatal(err)
	}
	s.move(ci, onCI)
	after := s.conversations("?archived=false")[0]
	if after.Cwd != onCI || after.ContextVersion <= before.ContextVersion {
		t.Fatalf("switch: %+v", after)
	}
	route := "/api/conversations/" + hostsConversation
	if got := s.directoryListing(route + "/fs").Path; got != onCI {
		t.Fatal(got)
	}
	attached := s.hosts()
	if len(attached) != 1 || attached[0].DeviceID != laptop.ID() || attached[0].Name != "laptop" || attached[0].Cwd == nil || *attached[0].Cwd != onLaptop || !attached[0].Online {
		t.Fatalf("attached: %+v", attached)
	}
	left := s.directoryListing(route + "/hosts/" + string(laptop.ID()) + "/fs")
	if len(left.Entries) != 1 || left.Entries[0].Name != "report.txt" {
		t.Fatalf("files: %+v", left)
	}
	s.move(laptop, onLaptop)
	attached = s.hosts()
	if len(attached) != 1 || attached[0].DeviceID != ci.ID() {
		t.Fatalf("attached: %+v", attached)
	}
	unchanged := s.conversations("?archived=false")[0]
	s.move(laptop, onLaptop)
	if s.conversations("?archived=false")[0].ContextVersion != unchanged.ContextVersion {
		t.Fatal("same target changes context")
	}
	s.refusal("PATCH", route, `{"target":{"kind":"workspace","workspaceId":"nowhere"}}`, 404, webapi.ErrorCodeWorkspaceNotFound)
	s.refusal("PATCH", route, `{"target":{"kind":"device","deviceId":"nothing","path":"/"}}`, 404, webapi.ErrorCodeDeviceNotFound)
	if err := s.h.AddUser(s.ctx, "user@example.test", "user-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	other, err := s.b.Login(s.ctx, "user@example.test", "user-pass-1")
	if err != nil {
		t.Fatal(err)
	}
	theirs, err := s.b.Pair(s.ctx, t, &other, "theirs")
	if err != nil {
		t.Fatal(err)
	}
	s.refusal("PATCH", route, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, theirs.ID(), s.directory(theirs, "x")), 404, webapi.ErrorCodeDeviceNotFound)
	s.request("PATCH", route, `{"archived":true}`, 200)
	s.refusal("PATCH", route, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, ci.ID(), onCI), 409, webapi.ErrorCodeConversationArchived)
}

// A 64 MiB response exceeds transport buffers so an unread download remains open at the switch.
func TestTargetSwitchCutsOpenDownload(t *testing.T) {
	s := newHostScenario(t, "")
	s.create(hostsConversation)
	laptop := s.pair("laptop")
	dir := s.directory(laptop, "work")
	s.move(laptop, dir)
	path := filepath.Join(dir, "long.mp4")
	if err := os.WriteFile(path, backendtest.Pattern(64*1024*1024, 0), 0644); err != nil {
		t.Fatal(err)
	}
	route := "/api/conversations/" + hostsConversation
	playing, err := s.b.Response(s.ctx, "GET", route+"/fs/raw?path="+url.QueryEscape(path), &s.user, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := playing.Body.Close(); err != nil {
			t.Error(err)
		}
	}()
	if playing.StatusCode != 200 {
		t.Fatal(playing.Status)
	}
	buffer := make([]byte, 1024)
	if n, err := playing.Body.Read(buffer); n == 0 || err != nil {
		t.Fatalf("first chunk: %d %v", n, err)
	}
	s.request("PATCH", route, `{"target":{"kind":"cloud"}}`, 200)
	if _, err := io.Copy(io.Discard, playing.Body); err == nil {
		t.Fatal("download completed instead of being cut")
	}
}

func TestAttachedHostsAreUniqueRenamableAndDetachable(t *testing.T) {
	s := newHostScenario(t, "")
	s.create(hostsConversation)
	laptop, ci, spare := s.pair("laptop"), s.pair("ci"), s.pair("ci")
	s.move(laptop, s.directory(laptop, "work"))
	route := "/api/conversations/" + hostsConversation + "/hosts"
	attach := func(device webapi.DeviceID, status int) backendtest.Answer {
		return s.request("POST", route, fmt.Sprintf(`{"deviceId":%q}`, device), status)
	}
	firstAnswer := attach(ci.ID(), 201)
	first, err := webapi.DecodeAttachedHosts(firstAnswer.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(first.Hosts) != 1 || first.Hosts[0].Name != "ci" || first.Hosts[0].Cwd != nil || !first.Hosts[0].Online {
		t.Fatalf("attached: %+v", first)
	}
	version := s.conversations("")[0].ContextVersion
	again := attach(ci.ID(), 201)
	repeated, err := webapi.DecodeAttachedHosts(again.Body)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(first, repeated); diff != "" {
		t.Fatal(diff)
	}
	if s.conversations("")[0].ContextVersion != version {
		t.Fatal("repeat attach changes context")
	}
	s.refusal("POST", route, fmt.Sprintf(`{"deviceId":%q}`, laptop.ID()), 409, webapi.ErrorCodeHostIsMain)
	s.refusal("POST", route, `{"deviceId":"nothing"}`, 404, webapi.ErrorCodeDeviceNotFound)
	attach(spare.ID(), 201)
	both := s.hosts()
	if len(both) != 2 || both[0].Name != "ci" || both[1].Name != "ci-2" {
		t.Fatalf("names: %+v", both)
	}
	renamed := s.request("PATCH", route+"/"+string(ci.ID()), `{"name":"  builder "}`, 200)
	hosts, err := webapi.DecodeAttachedHosts(renamed.Body)
	if err != nil {
		t.Fatal(err)
	}
	if hosts.Hosts[0].Name != "builder" {
		t.Fatal(hosts)
	}
	s.refusal("PATCH", route+"/"+string(spare.ID()), `{"name":"builder"}`, 409, webapi.ErrorCodeNameTaken)
	s.refusal("PATCH", route+"/"+string(laptop.ID()), `{"name":"main"}`, 404, webapi.ErrorCodeHostNotAttached)
	s.refusal("PATCH", route+"/"+string(ci.ID()), `{"name":"   "}`, 400, webapi.ErrorCodeInvalidBody)
	s.request("DELETE", route+"/"+string(ci.ID()), "", 204)
	left := s.hosts()
	if len(left) != 1 || left[0].DeviceID != spare.ID() {
		t.Fatal(left)
	}
	s.request("DELETE", route+"/"+string(ci.ID()), "", 204)
	s.refusal("GET", route+"/"+string(ci.ID())+"/fs", "", 404, webapi.ErrorCodeHostNotAttached)
}

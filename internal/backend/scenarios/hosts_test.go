package scenarios_test

import (
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/webapiproto"
)

const hostsConversation = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01"

func (s *hostScenario) directory(device *backendtest.Paired, name string) string {
	s.t.Helper()
	path := filepath.Join(device.Runner.Home(), name)
	if err := os.MkdirAll(path, 0o755); err != nil {
		s.t.Fatal(err)
	}
	return path
}

func (s *hostScenario) move(device *backendtest.Paired, path string) backendtest.Answer {
	s.t.Helper()
	return conversationRequest(
		s.ctx,
		s.t,
		s.b,
		&s.user,
		"PATCH",
		"/api/conversations/"+hostsConversation,
		fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, device.ID(), path),
		200,
	)
}

func (s *hostScenario) hosts() []webapiproto.AttachedHost {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+hostsConversation+"/hosts", "", 200)
	hosts, err := webapiproto.DecodeAttachedHosts(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return hosts.Hosts
}

func (s *hostScenario) directoryListing(path string) webapiproto.Directory {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", path, "", 200)
	d, err := webapiproto.DecodeDirectory(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return d
}

// TestTargetSwitchAttachesPreviousDevice checks previous-device attachment when switching targets.
// Pairing real runners and reading both devices proves the switch moves access without moving files.
func TestTargetSwitchAttachesPreviousDevice(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	laptop, ci := s.pair("laptop"), s.pair("ci")
	onLaptop, onCI := s.directory(laptop, "work"), s.directory(ci, "build")
	moved := s.move(laptop, onLaptop)
	update, err := webapiproto.DecodeConversationUpdate(moved.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(update.Results) != 1 {
		t.Fatalf("results: %+v", update.Results)
	}
	applied, ok := update.Results[0].(*webapiproto.FieldResultApplied)
	if !ok || applied.Field != webapiproto.PatchField("target") {
		t.Fatalf("target result: %+v", update.Results)
	}
	before := s.conversations("?archived=false")[0]
	if before.Cwd != onLaptop {
		t.Fatal(before.Cwd)
	}
	if len(s.hosts()) != 0 {
		t.Fatal("unused Cloud attached")
	}
	if err := os.WriteFile(filepath.Join(onLaptop, "report.txt"), []byte("on the laptop"), 0o644); err != nil {
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
	if len(attached) != 1 || attached[0].DeviceID != laptop.ID() || attached[0].Name != "laptop" ||
		attached[0].Cwd == nil ||
		*attached[0].Cwd != onLaptop ||
		!attached[0].Online {
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
	conversationRefusal(
		s.t,
		conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"PATCH",
			route,
			`{"target":{"kind":"workspace","workspaceId":"nowhere"}}`,
			404,
		),
		webapiproto.ErrorCodeWorkspaceNotFound,
	)
	conversationRefusal(
		s.t,
		conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"PATCH",
			route,
			`{"target":{"kind":"device","deviceId":"nothing","path":"/"}}`,
			404,
		),
		webapiproto.ErrorCodeDeviceNotFound,
	)
	if err := s.h.AddUser(s.ctx, "user@example.test", "user-pass-1", webapiproto.RoleUser); err != nil {
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
	conversationRefusal(
		s.t,
		conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"PATCH",
			route,
			fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, theirs.ID(), s.directory(theirs, "x")),
			404,
		),
		webapiproto.ErrorCodeDeviceNotFound,
	)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route, `{"archived":true}`, 200)
	conversationRefusal(
		s.t,
		conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"PATCH",
			route,
			fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, ci.ID(), onCI),
			409,
		),
		webapiproto.ErrorCodeConversationArchived,
	)
}

// TestTargetSwitchCutsOpenDownload checks cancellation of an open download when switching targets.
// A 64 MiB response exceeds transport buffers so an unread download remains open at the switch.
func TestTargetSwitchCutsOpenDownload(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	laptop := s.pair("laptop")
	directory := s.directory(laptop, "work")
	s.move(laptop, directory)
	path := filepath.Join(directory, "long.mp4")
	if err := os.WriteFile(path, backendtest.Pattern(64*1024*1024, 0), 0o644); err != nil {
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
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route, `{"target":{"kind":"cloud"}}`, 200)
	if _, err := io.Copy(io.Discard, playing.Body); err == nil {
		t.Fatal("download completed instead of being cut")
	}
}

// TestAttachedHostsAreUniqueRenamableAndDetachable checks attached host uniqueness, renaming and detachment.
func TestAttachedHostsAreUniqueRenamableAndDetachable(t *testing.T) {
	t.Parallel()
	s := newHostScenario(t, "")
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	laptop, ci, spare := s.pair("laptop"), s.pair("ci"), s.pair("ci")
	s.move(laptop, s.directory(laptop, "work"))
	route := "/api/conversations/" + hostsConversation + "/hosts"
	attach := func(device webapiproto.DeviceID, status int) backendtest.Answer {
		return conversationRequest(
			s.ctx,
			s.t,
			s.b,
			&s.user,
			"POST",
			route,
			fmt.Sprintf(`{"deviceId":%q}`, device),
			status,
		)
	}
	firstAnswer := attach(ci.ID(), 201)
	first, err := webapiproto.DecodeAttachedHosts(firstAnswer.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(first.Hosts) != 1 || first.Hosts[0].Name != "ci" || first.Hosts[0].Cwd != nil || !first.Hosts[0].Online {
		t.Fatalf("attached: %+v", first)
	}
	version := s.conversations("")[0].ContextVersion
	again := attach(ci.ID(), 201)
	repeated, err := webapiproto.DecodeAttachedHosts(again.Body)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(first, repeated); diff != "" {
		t.Fatal(diff)
	}
	if s.conversations("")[0].ContextVersion != version {
		t.Fatal("repeat attach changes context")
	}
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", route, fmt.Sprintf(`{"deviceId":%q}`, laptop.ID()), 409),
		webapiproto.ErrorCodeHostIsMain,
	)
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", route, `{"deviceId":"nothing"}`, 404),
		webapiproto.ErrorCodeDeviceNotFound,
	)
	secondAnswer := attach(spare.ID(), 201)
	second, err := webapiproto.DecodeAttachedHosts(secondAnswer.Body)
	if err != nil {
		t.Fatal(err)
	}
	both := second.Hosts
	if len(both) != 2 || both[0].Name != "ci" || both[1].Name != "ci-2" {
		t.Fatalf("names: %+v", both)
	}
	renamed := conversationRequest(
		s.ctx,
		s.t,
		s.b,
		&s.user,
		"PATCH",
		route+"/"+string(ci.ID()),
		`{"name":"  builder "}`,
		200,
	)
	hosts, err := webapiproto.DecodeAttachedHosts(renamed.Body)
	if err != nil {
		t.Fatal(err)
	}
	if hosts.Hosts[0].Name != "builder" {
		t.Fatal(hosts)
	}
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route+"/"+string(spare.ID()), `{"name":"builder"}`, 409),
		webapiproto.ErrorCodeNameTaken,
	)
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route+"/"+string(laptop.ID()), `{"name":"main"}`, 404),
		webapiproto.ErrorCodeHostNotAttached,
	)
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route+"/"+string(ci.ID()), `{"name":"   "}`, 400),
		webapiproto.ErrorCodeInvalidBody,
	)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route+"/"+string(ci.ID()), "", 204)
	left := s.hosts()
	if len(left) != 1 || left[0].DeviceID != spare.ID() {
		t.Fatal(left)
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route+"/"+string(ci.ID()), "", 204)
	conversationRefusal(
		s.t,
		conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", route+"/"+string(ci.ID())+"/fs", "", 404),
		webapiproto.ErrorCodeHostNotAttached,
	)
}

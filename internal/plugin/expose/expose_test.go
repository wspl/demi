package expose_test

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/expose"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

type testWorld struct {
	*plugintest.TestDemi
	plugin plugin.Plugin
	root   commanddecl.Node[commanddecl.Binding]
}

func world(t *testing.T) *testWorld {
	t.Helper()
	d := plugintest.New()
	d.Hosts = []plugin.ConversationHost{
		{Name: "laptop", Device: "device-laptop", Role: plugin.HostRoleMain, Online: true},
		{Name: "ci", Device: "device-ci", Role: plugin.HostRoleAttached, Online: true},
	}
	return over(t, d)
}

func over(t *testing.T, d *plugintest.TestDemi) *testWorld {
	t.Helper()
	factory, err := expose.New()
	if err != nil {
		t.Fatal(err)
	}
	roots, err := plugintest.Roots(factory.Manifest())
	if err != nil {
		t.Fatal(err)
	}
	return &testWorld{TestDemi: d, plugin: plugintest.Loopback(factory.Instance()), root: roots[0]}
}

func minutes(t *testing.T, n int64) types.Timestamp {
	t.Helper()
	value, err := types.TimestampFromMillisecond(n * 60000)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

// command follows the runner's argv parsing and the plugin host's JSON boundary.
func command(t *testing.T, d *testWorld, line ...string) (uint8, string, string) {
	t.Helper()
	parsed, err := plugintest.Parse(d.root, line, nil)
	if err != nil {
		t.Fatal(err)
	}
	args, err := parsed.Values.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	memory := hosttest.NewMemoryPort(nil)
	d.RPC = memory
	reply, err := d.plugin.Call(t.Context(), &plugin.RequestCommand{User: "u1", Invocation: host.RPCInvocation{
		Path:    parsed.Path[1:],
		Argv:    line,
		Args:    args,
		JSON:    parsed.JSON,
		CWD:     "/workspace",
		Env:     map[string]string{},
		Context: hosttest.CommandContext(),
	}}, d.Port())
	if err != nil {
		t.Fatal(err)
	}
	exit, ok := reply.(*plugin.ReplyExit)
	if !ok {
		t.Fatalf("unexpected reply: %T", reply)
	}
	return exit.Code, string(memory.Stdout()), string(memory.Stderr())
}

func pageState(t *testing.T, d *testWorld) expose.ExposeState {
	t.Helper()
	reply, err := d.plugin.Call(t.Context(), &plugin.RequestPageState{User: "u1"}, d.Port())
	if err != nil {
		t.Fatal(err)
	}
	state, ok := reply.(*plugin.ReplyState)
	if !ok {
		t.Fatalf("unexpected reply: %T", reply)
	}
	value, err := expose.DecodeExposeState(state.State)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func pageCall(t *testing.T, d *testWorld, method string, params json.RawMessage) (json.RawMessage, error) {
	t.Helper()
	reply, err := d.plugin.Call(
		t.Context(),
		&plugin.RequestPageCall{User: "u1", Method: method, Params: params},
		d.Port(),
	)
	if err != nil {
		return nil, err
	}
	result, ok := reply.(*plugin.ReplyResult)
	if !ok {
		t.Fatalf("unexpected reply: %T", reply)
	}
	return result.Result, nil
}

// In-memory scenario, under one second; no timers or external resources.
func TestExposesKnownByNumbersNeverGivenTwice(t *testing.T) {
	d := world(t)
	code, added, _ := command(t, d, "expose", "add", "5173")
	url := d.LiveExposes()[0].URL
	want := fmt.Sprintf("Exposed 127.0.0.1:5173 on laptop as %s\nExpires in 60 minutes (expose 1).\n", url)
	if code != 0 || added != want {
		t.Fatalf("%d %q, want %q", code, added, want)
	}
	d.Now = minutes(t, 1)
	code, added, _ = command(t, d, "expose", "add", "127.0.0.1:8080", "--host", "ci")
	if code != 0 || !strings.Contains(added, "on ci as") || !strings.HasSuffix(added, "(expose 2).\n") {
		t.Fatalf("%d %q", code, added)
	}
	d.Now = minutes(t, 2)
	_, listed, _ := command(t, d, "expose", "list")
	want = fmt.Sprintf(
		"Expose  Device  Address         Expires  URL\n"+
			"1       laptop  127.0.0.1:5173  58 min   %s\n"+
			"2       ci      127.0.0.1:8080  59 min   %s\n",
		url,
		d.LiveExposes()[1].URL,
	)
	if listed != want {
		t.Fatalf("list = %q, want %q", listed, want)
	}
	_, listed, _ = command(t, d, "expose", "list", "--json")
	lines, err := expose.DecodeExposeLines([]byte(listed))
	if err != nil {
		t.Fatal(err)
	}
	if len(lines.Exposes) != 2 || lines.Exposes[0].Number != 1 || lines.Exposes[1].Device != "ci" ||
		strings.Contains(listed, `"id"`) {
		t.Fatal(listed)
	}
	_, renewed, _ := command(t, d, "expose", "renew", "1")
	if renewed != "Expose 1 expires in 60 minutes.\n" {
		t.Fatal(renewed)
	}
	_, removed, _ := command(t, d, "expose", "remove", "1")
	if removed != "Removed expose 1; its URL no longer works.\n" {
		t.Fatal(removed)
	}
	for _, leaf := range []string{"renew", "remove"} {
		code, _, refused := command(t, d, "expose", leaf, "1")
		if code != 1 || refused != "expose "+leaf+": no expose 1\n" {
			t.Fatalf("%d %q", code, refused)
		}
	}
	// A new factory and instance after restart retain the next persisted number.
	d = over(t, d.TestDemi)
	_, added, _ = command(t, d, "expose", "add", "3000")
	if !strings.HasSuffix(added, "(expose 3).\n") {
		t.Fatal(added)
	}
}

func TestAddRefusals(t *testing.T) {
	d := world(t)
	d.Hosts[1].Online = false
	for _, tc := range []struct {
		args   []string
		reason string
	}{
		{
			[]string{
				"expose",
				"add",
				"8080",
				"--host",
				"nope",
			},
			"host nope is not reachable from this conversation (see `demi host list`)",
		},
		{[]string{"expose", "add", "8080", "--host", "ci"}, "the device is offline; connect it before exposing a service"},
		{[]string{"expose", "add", "localhost"}, "must be host:port or a port, the port 1 to 65535"},
	} {
		code, _, refused := command(t, d, tc.args...)
		if code != 1 || refused != "expose add: "+tc.reason+"\n" {
			t.Fatalf("%d %q", code, refused)
		}
	}
	d.ExposesAvailable = false
	code, _, refused := command(t, d, "expose", "add", "8080")
	if code != 1 || refused != "expose add: exposes are not available on this instance\n" {
		t.Fatalf("%d %q", code, refused)
	}
	s := pageState(t, d)
	if s.Available || len(s.Exposes) != 0 {
		t.Fatal(s)
	}
}

func TestPageStateRenewAndRemoveByID(t *testing.T) {
	d := world(t)
	command(t, d, "expose", "add", "5173")
	record := d.LiveExposes()[0]
	s := pageState(t, d)
	if !s.Available || len(s.Exposes) != 1 {
		t.Fatal(s)
	}
	entry := s.Exposes[0]
	if entry.ID != record.ID || entry.Number != 1 || entry.DeviceID != "device-laptop" ||
		entry.DeviceName != "laptop" ||
		entry.Address != "127.0.0.1:5173" ||
		entry.URL != record.URL ||
		entry.ExpiresAt != minutes(t, 60) {
		t.Fatal(entry)
	}
	d.Now = minutes(t, 30)
	params, err := (expose.ExposeCall{Expose: string(record.ID)}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	result, err := pageCall(t, d, "renew", params)
	if err != nil || string(result) != "null" {
		t.Fatalf("%s %v", result, err)
	}
	if pageState(t, d).Exposes[0].ExpiresAt != minutes(t, 90) {
		t.Fatal("renew did not extend expiry")
	}
	result, err = pageCall(t, d, "remove", params)
	if err != nil || string(result) != "null" {
		t.Fatalf("%s %v", result, err)
	}
	if len(pageState(t, d).Exposes) != 0 {
		t.Fatal("remove left expose live")
	}
	for _, id := range []string{string(record.ID), "not-an-id"} {
		params, err = (expose.ExposeCall{Expose: id}).MarshalJSON()
		if err != nil {
			t.Fatal(err)
		}
		_, err = pageCall(t, d, "renew", params)
		var refusal *plugin.ErrorRefused
		if !errors.As(err, &refusal) || refusal.Reason != "expose_not_found" {
			t.Fatalf("%v", err)
		}
	}
}

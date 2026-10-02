package expose

import (
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func world() *plugintest.TestDemi {
	d := plugintest.New()
	d.Hosts = []plugin.ConversationHost{
		{Name: "laptop", Device: "device-laptop", Role: plugin.HostRoleMain, Online: true},
		{Name: "ci", Device: "device-ci", Role: plugin.HostRoleAttached, Online: true},
	}
	return d
}

func minutes(t *testing.T, n int64) core.Timestamp {
	t.Helper()
	value, err := core.TimestampFromMillisecond(n * 60000)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

// command exercises the real port handlers; declaration/argv coverage awaits the factory.
func command(t *testing.T, d *plugintest.TestDemi, leaf, args string, jsonOutput bool) (uint8, string, string) {
	t.Helper()
	memory := hosttest.NewMemoryPort(nil)
	d.RPC = memory
	code, err := run(t.Context(), host.RPCInvocation{Path: []string{"expose", leaf}, Args: []byte(args), JSON: jsonOutput}, d.Port())
	if err != nil {
		t.Fatal(err)
	}
	return code, string(memory.Stdout()), string(memory.Stderr())
}

func pageState(t *testing.T, d *plugintest.TestDemi) ExposeState {
	t.Helper()
	data, err := state(t.Context(), d.Port())
	if err != nil {
		t.Fatal(err)
	}
	value, err := DecodeExposeState(data)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

// In-memory scenario, under one second; no timers or external resources.
func TestExposesKnownByNumbersNeverGivenTwice(t *testing.T) {
	d := world()
	code, added, _ := command(t, d, "add", `{"address":"5173"}`, false)
	url := d.LiveExposes()[0].URL
	want := fmt.Sprintf("Exposed 127.0.0.1:5173 on laptop as %s\nExpires in 60 minutes (expose 1).\n", url)
	if code != 0 || added != want {
		t.Fatalf("%d %q, want %q", code, added, want)
	}
	d.Now = minutes(t, 1)
	code, added, _ = command(t, d, "add", `{"address":"127.0.0.1:8080","host":"ci"}`, false)
	if code != 0 || !strings.Contains(added, "on ci as") || !strings.HasSuffix(added, "(expose 2).\n") {
		t.Fatalf("%d %q", code, added)
	}
	d.Now = minutes(t, 2)
	_, listed, _ := command(t, d, "list", `{}`, false)
	want = fmt.Sprintf("Expose  Device  Address         Expires  URL\n1       laptop  127.0.0.1:5173  58 min   %s\n2       ci      127.0.0.1:8080  59 min   %s\n", url, d.LiveExposes()[1].URL)
	if listed != want {
		t.Fatalf("list = %q, want %q", listed, want)
	}
	_, listed, _ = command(t, d, "list", `{}`, true)
	lines, err := DecodeExposeLines([]byte(listed))
	if err != nil {
		t.Fatal(err)
	}
	if len(lines.Exposes) != 2 || lines.Exposes[0].Number != 1 || lines.Exposes[1].Device != "ci" || strings.Contains(listed, `"id"`) {
		t.Fatal(listed)
	}
	_, renewed, _ := command(t, d, "renew", `{"number":1}`, false)
	if renewed != "Expose 1 expires in 60 minutes.\n" {
		t.Fatal(renewed)
	}
	_, removed, _ := command(t, d, "remove", `{"number":1}`, false)
	if removed != "Removed expose 1; its URL no longer works.\n" {
		t.Fatal(removed)
	}
	for _, leaf := range []string{"renew", "remove"} {
		code, _, refused := command(t, d, leaf, `{"number":1}`, false)
		if code != 1 || refused != "expose "+leaf+": no expose 1\n" {
			t.Fatalf("%d %q", code, refused)
		}
	}
	// Handlers have no retained state; the same persisted port is all a fresh instance needs.
	_, added, _ = command(t, d, "add", `{"address":"3000"}`, false)
	if !strings.HasSuffix(added, "(expose 3).\n") {
		t.Fatal(added)
	}
}

func TestAddRefusals(t *testing.T) {
	d := world()
	d.Hosts[1].Online = false
	for _, tc := range []struct{ args, reason string }{
		{`{"address":"8080","host":"nope"}`, "host nope is not reachable from this conversation (see `demi host list`)"},
		{`{"address":"8080","host":"ci"}`, "the device is offline; connect it before exposing a service"},
		{`{"address":"localhost"}`, "must be host:port or a port, the port 1 to 65535"},
	} {
		code, _, refused := command(t, d, "add", tc.args, false)
		if code != 1 || refused != "expose add: "+tc.reason+"\n" {
			t.Fatalf("%d %q", code, refused)
		}
	}
	d.ExposesAvailable = false
	code, _, refused := command(t, d, "add", `{"address":"8080"}`, false)
	if code != 1 || refused != "expose add: exposes are not available on this instance\n" {
		t.Fatalf("%d %q", code, refused)
	}
	s := pageState(t, d)
	if s.Available || len(s.Exposes) != 0 {
		t.Fatal(s)
	}
}

func TestPageStateRenewAndRemoveByID(t *testing.T) {
	d := world()
	command(t, d, "add", `{"address":"5173"}`, false)
	record := d.LiveExposes()[0]
	s := pageState(t, d)
	if !s.Available || len(s.Exposes) != 1 {
		t.Fatal(s)
	}
	entry := s.Exposes[0]
	if entry.ID != record.ID || entry.Number != 1 || entry.DeviceID != "device-laptop" || entry.DeviceName != "laptop" || entry.Address != "127.0.0.1:5173" || entry.URL != record.URL || entry.ExpiresAt != minutes(t, 60) {
		t.Fatal(entry)
	}
	d.Now = minutes(t, 30)
	params, err := (ExposeCall{Expose: string(record.ID)}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	result, err := pageCall(t.Context(), "renew", params, d.Port())
	if err != nil || string(result) != "null" {
		t.Fatalf("%s %v", result, err)
	}
	if pageState(t, d).Exposes[0].ExpiresAt != minutes(t, 90) {
		t.Fatal("renew did not extend expiry")
	}
	result, err = pageCall(t.Context(), "remove", params, d.Port())
	if err != nil || string(result) != "null" {
		t.Fatalf("%s %v", result, err)
	}
	if len(pageState(t, d).Exposes) != 0 {
		t.Fatal("remove left expose live")
	}
	for _, id := range []string{string(record.ID), "not-an-id"} {
		params, err = (ExposeCall{Expose: id}).MarshalJSON()
		if err != nil {
			t.Fatal(err)
		}
		_, err = pageCall(t.Context(), "renew", params, d.Port())
		var refusal *plugin.ErrorRefused
		if !errors.As(err, &refusal) || refusal.Reason != "expose_not_found" {
			t.Fatalf("%v", err)
		}
	}
}

package runnerproto_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/go/runnerproto"
)

func TestABootRecordIsCheckedWhereItIsRead(t *testing.T) {
	boot, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"HTTP://Backend:80","deviceToken":"opaque"}`))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := boot.Encode()
	if want := `{"backendUrl":"http://backend/","deviceToken":"opaque"}`; err != nil || string(encoded) != want {
		t.Errorf("encoded %s, %v; want %s", encoded, err, want)
	}
	for record, want := range map[string]string{
		`{"backendUrl":"http://b","deviceToken":"t","command":"x"}`: "invalid: command: unknown member",
		`{"backendUrl":"ftp://b","deviceToken":"t"}`:                "invalid: backendUrl: is not a backend URL",
		`{"backendUrl":"http://u:p@b","deviceToken":"t"}`:           "invalid: backendUrl: is not a backend URL",
		`{"backendUrl":"http://b/#","deviceToken":"t"}`:             "invalid: backendUrl: is not a backend URL",
		`{"backendUrl":"http://b/#f","deviceToken":"t"}`:            "invalid: backendUrl: is not a backend URL",
		`{"backendUrl":"http://b:65536","deviceToken":"t"}`:         "invalid: backendUrl: is not a URL",
		`{"backendUrl":"http://b","deviceToken":"two words"}`:       "invalid: deviceToken: is not a device token",
		`{"backendUrl":"http://b","deviceToken":""}`:                "invalid: deviceToken: is not a device token",
		`{"backendUrl":"http://b"}`:                                 "invalid: deviceToken: required",
	} {
		if _, err := runnerproto.DecodeManagedBoot([]byte(record)); err == nil || err.Error() != want {
			t.Errorf("%s: %v, want %q", record, err, want)
		}
	}
	long := strings.Repeat("a", 4097)
	if _, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"http://b","deviceToken":"` + long + `"}`)); err == nil {
		t.Error("a token of 4097 units was accepted")
	}
	// The limit counts UTF-16 units: an emoji is two.
	units := strings.Repeat("a", 4095) + "😀"
	if _, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"http://b","deviceToken":"` + units + `"}`)); err == nil {
		t.Error("a token of 4097 UTF-16 units was accepted")
	}
}

func TestATokenNeverAppearsInFormattedOutput(t *testing.T) {
	boot, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"http://b","deviceToken":"dt_secret"}`))
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%v", "%+v", "%#v", "%s"} {
		if text := fmt.Sprintf(format, boot); strings.Contains(text, "dt_secret") {
			t.Errorf("%s shows the token: %s", format, text)
		}
	}
}

// Cost: in-process parsing and codecs only; no timers or external services.
func TestManagedBootUsesWHATWGNormalization(t *testing.T) {
	for raw, want := range map[string]string{
		"HTTPS://Example.COM:443/a/../b": "https://example.com/b",
		"ws://Example.COM:80/./a":        "ws://example.com/a",
		"https://bücher.example/":        "https://xn--bcher-kva.example/",
		"http://example.com/a/%2e%2e/b":  "http://example.com/b",
	} {
		boot := runnerproto.ManagedBoot{BackendURL: runnerproto.BackendURL(raw), DeviceToken: "opaque"}
		json, err := boot.Encode()
		expected := `{"backendUrl":"` + want + `","deviceToken":"opaque"}`
		if err != nil || string(json) != expected {
			t.Errorf("%s: %s %v; want %s", raw, json, err, expected)
		}
		packed, err := boot.MarshalMsgpack()
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := runnerproto.DecodeManagedBootMsgpack(packed)
		if err != nil || string(decoded.BackendURL) != want {
			t.Errorf("MessagePack: %+v %v", decoded, err)
		}
	}
	for _, raw := range []string{"https://example.com/#", "https://user:pass@example.com/", "ftp://example.com/"} {
		boot := runnerproto.ManagedBoot{BackendURL: runnerproto.BackendURL(raw), DeviceToken: "opaque"}
		if _, err := boot.Encode(); err == nil {
			t.Errorf("accepted %s", raw)
		}
		if _, err := boot.MarshalMsgpack(); err == nil {
			t.Errorf("MessagePack accepted %s", raw)
		}
	}
}

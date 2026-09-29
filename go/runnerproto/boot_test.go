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

package claudecodeproto_test

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/contract"
)

// These pure boundary checks use no IO or waits and cost well under one second.
func TestReleaseChecksEveryEntry(t *testing.T) {
	digest := strings.Repeat("a", 64)
	for _, tc := range []struct {
		name    string
		version string
		url     string
		size    uint64
		digest  string
		valid   bool
	}{
		{"valid", "2.1.3-beta.1", "https://example.test/claude", 3, digest, true},
		{"short version", "2.1", "https://example.test/claude", 3, digest, false},
		{"invalid URL", "2.1.3", "not a url", 3, digest, false},
		{"empty artifact", "2.1.3", "https://example.test/claude", 0, digest, false},
		{"uppercase digest", "2.1.3", "https://example.test/claude", 3, strings.Repeat("A", 64), false},
		{"short digest", "2.1.3", "https://example.test/claude", 3, "abc", false},
		{"non HTTP URL", "2.1.3", "ftp://example.test/claude", 3, digest, true},
		{"full size range", "2.1.3", "https://example.test/claude", ^uint64(0), digest, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			// A second, valid platform ensures all entries are checked, regardless
			// of which target the eventual caller runs on.
			input := fmt.Sprintf(
				`{"version":%q,"platforms":{"darwin-arm64":{"url":%q,"size":%d,"sha256":%q},`+
					`"linux-arm64":{"url":"https://example.test/claude","size":3,"sha256":%q}}}`,
				tc.version,
				tc.url,
				tc.size,
				tc.digest,
				digest,
			)
			release, err := claudecodeproto.DecodeRelease([]byte(input))
			if (err == nil) != tc.valid {
				t.Fatalf("DecodeRelease() error = %v, want valid %v", err, tc.valid)
			}
			if tc.valid && release.Platforms["darwin-arm64"].Size != tc.size {
				t.Fatalf("size = %d, want %d", release.Platforms["darwin-arm64"].Size, tc.size)
			}
		})
	}
	for _, input := range []string{
		`"not a record"`,
		`{`,
		`{"version":"2.1.3","platforms":{"linux-x64":{"url":"https://example.test/claude","size":3,` +
			`"sha256":"` + digest + `","extra":1}}}`,
		`{"version":"2.1.3","platforms":{},"extra":1}`,
		`{"version":"2.1.3"}`,
		`{"version":"2.1.3","platforms":null}`,
		`{"version":"2.1.3","version":"2.1.4","platforms":{}}`,
	} {
		if _, err := claudecodeproto.DecodeRelease([]byte(input)); err == nil {
			t.Errorf("DecodeRelease(%s) succeeded", input)
		}
	}
}

func TestVersionDirectoryNames(t *testing.T) {
	for _, valid := range []string{"1.0.0", "10.20.30-rc.1", "0.0.1-alpha-2", "18446744073709551615.0.0"} {
		if _, err := claudecodeproto.DecodeVersion([]byte(fmt.Sprintf("%q", valid))); err != nil {
			t.Errorf("version %q: %v", valid, err)
		}
	}
	for _, invalid := range []string{
		"1.0", "1.0.0-", "1.0.0+build", "v1.0.0", "1..0", "../1.0.0", "01.0.0",
		"1", "1.0.0-01", "18446744073709551616.0.0", "1.18446744073709551616.0", "1.0.18446744073709551616",
		"", "1.0.0.1", "1.0.x", "1.0.0/..", "1.0.0-a/b", " 1.0.0",
	} {
		if _, err := claudecodeproto.DecodeVersion([]byte(fmt.Sprintf("%q", invalid))); err == nil {
			t.Errorf("version %q was accepted", invalid)
		}
	}
}

// The service stream exposes only stdout, so both outcomes must carry their
// boolean discriminator alongside the answer. This test uses no IO or waits.
func TestRepliesCarryOKBesideAnswer(t *testing.T) {
	installed := claudecodeproto.Installed{Version: "2.1.3", Path: "/opt/claude"}
	failure := &claudecodeproto.Failed{Failure: claudecodeproto.Failure{
		Code: claudecodeproto.InstallFailed, Message: "digest differs",
	}}
	for _, tc := range []struct {
		name  string
		reply claudecodeproto.StatusReply
		wire  string
	}{
		{"done", &claudecodeproto.StatusDone{Status: claudecodeproto.Status{
			Platform: "darwin-arm64", Installed: []claudecodeproto.Installed{installed},
		}}, `{"ok":true,"platform":"darwin-arm64","installed":[{"version":"2.1.3","path":"/opt/claude"}]}`},
		{"failed", failure, `{"ok":false,"code":"install_failed","message":"digest differs"}`},
	} {
		t.Run("status/"+tc.name, func(t *testing.T) {
			printed, err := contract.EncodeJSON(claudecodeproto.StatusReplyJSON{Value: tc.reply})
			if err != nil {
				t.Fatal(err)
			}
			if string(printed) != tc.wire {
				t.Fatalf("encoded %s, want %s", printed, tc.wire)
			}
			decoded, err := claudecodeproto.DecodeStatusReply(printed)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(decoded, tc.reply) {
				t.Fatalf("decoded %#v, want %#v", decoded, tc.reply)
			}
		})
	}
	for _, tc := range []struct {
		name  string
		reply claudecodeproto.EnsureReply
		wire  string
	}{
		{"done", &claudecodeproto.Ensured{Installed: installed}, `{"ok":true,"version":"2.1.3","path":"/opt/claude"}`},
		{"failed", failure, `{"ok":false,"code":"install_failed","message":"digest differs"}`},
	} {
		t.Run("ensure/"+tc.name, func(t *testing.T) {
			printed, err := contract.EncodeJSON(claudecodeproto.EnsureReplyJSON{Value: tc.reply})
			if err != nil {
				t.Fatal(err)
			}
			if string(printed) != tc.wire {
				t.Fatalf("encoded %s, want %s", printed, tc.wire)
			}
			decoded, err := claudecodeproto.DecodeEnsureReply(printed)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(decoded, tc.reply) {
				t.Fatalf("decoded %#v, want %#v", decoded, tc.reply)
			}
		})
	}
	for _, wire := range []string{
		`{"platform":"x","installed":[]}`,
		`{"ok":"true","platform":"x","installed":[]}`,
		`{"ok":null,"platform":"x","installed":[]}`,
		`{"ok":true,"platform":"x","installed":[],"code":"install_failed","message":"mixed"}`,
		`{"ok":false,"code":"unknown","message":"failed"}`,
		`{"ok":false,"code":"install_failed"}`,
		`{"ok":true,"version":"2.1.3"}`,
	} {
		if _, err := claudecodeproto.DecodeStatusReply([]byte(wire)); err == nil {
			t.Errorf("DecodeStatusReply(%s) succeeded", wire)
		}
		if _, err := claudecodeproto.DecodeEnsureReply([]byte(wire)); err == nil {
			t.Errorf("DecodeEnsureReply(%s) succeeded", wire)
		}
	}
}

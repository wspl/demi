package claudeproto_test

import (
	"encoding/json/v2"
	"errors"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/claudeproto"
)

func record(version, url string, size uint64, sha256 string) []byte {
	data, err := json.Marshal(map[string]any{
		"version":   version,
		"platforms": map[string]any{"darwin-arm64": map[string]any{"url": url, "size": size, "sha256": sha256}},
	})
	if err != nil {
		panic(err)
	}
	return data
}

func TestAReleaseRecordIsCheckedInEveryEntry(t *testing.T) {
	digest := strings.Repeat("a", 64)
	release, err := claudeproto.Decode[claudeproto.Release](record("2.1.3-beta.1", "https://example.test/claude", 3, digest))
	if err != nil || release.Platforms["darwin-arm64"].Size != 3 {
		t.Fatalf("a valid record: %+v, %v", release, err)
	}
	// The largest port, and a scheme that names no host.
	for _, url := range []string{"https://example.test:65535/claude", "https://[::1]:8443/claude", "file:///claude"} {
		if _, err := claudeproto.Decode[claudeproto.Release](record("2.1.3", url, 3, digest)); err != nil {
			t.Errorf("%s was refused: %v", url, err)
		}
	}
	for name, test := range map[string]struct {
		record []byte
		want   string
	}{
		"a version that is not SemVer": {record("2.1", "https://example.test/claude", 3, digest), "invalid: version: is not a SemVer version without build metadata"},
		"a URL that is none":           {record("2.1.3", "not a url", 3, digest), `invalid: platforms["darwin-arm64"].url: is not a URL`},
		"an https URL without a host":  {record("2.1.3", "https://", 3, digest), `invalid: platforms["darwin-arm64"].url: is not a URL`},
		"an http URL without a host":   {record("2.1.3", "http:///claude", 3, digest), `invalid: platforms["darwin-arm64"].url: is not a URL`},
		"a port over 65535":            {record("2.1.3", "https://example.test:65536/claude", 3, digest), `invalid: platforms["darwin-arm64"].url: is not a URL`},
		"a size of nothing":            {record("2.1.3", "https://example.test/claude", 0, digest), `invalid: platforms["darwin-arm64"].size: must be at least 1`},
		"a digest in capitals":         {record("2.1.3", "https://example.test/claude", 3, strings.Repeat("A", 64)), `invalid: platforms["darwin-arm64"].sha256: must match ^[a-f0-9]{64}$`},
		"a member the record lacks":    {[]byte(`{"version": "2.1.3", "platforms": {}, "extra": 1}`), "invalid: extra: unknown member"},
		"no platforms":                 {[]byte(`{"version": "2.1.3"}`), "invalid: platforms: required"},
		"an entry with a member more":  {[]byte(`{"version":"2.1.3","platforms":{"linux-x64":{"url":"https://h/c","size":1,"sha256":"` + digest + `","extra":1}}}`), `invalid: platforms["linux-x64"].extra: unknown member`},
	} {
		_, err := claudeproto.Decode[claudeproto.Release](test.record)
		var invalid *claudeproto.InvalidError
		if !errors.As(err, &invalid) || err.Error() != test.want {
			t.Errorf("%s: %v, want %q", name, err, test.want)
		}
	}
}

func TestVersionsAreThreeNumbersAndAnOptionalPrerelease(t *testing.T) {
	for _, valid := range []string{"1.0.0", "10.20.30-rc.1", "0.0.1-alpha-2", "2.1.278", "0.0.0", "1.0.0-rc-1", "1.0.0-0", "1.0.0-alpha.beta.1", "18446744073709551615.0.0"} {
		if !claudeproto.IsVersion(valid) {
			t.Errorf("%q was refused", valid)
		}
	}
	for _, invalid := range []string{
		"", "1.0", "1.0.0-", "1.0.0+build", "v1.0.0", "1..0", "../1.0.0", "01.0.0", "2.1.278.1", "2.1.x", "2.1.278/..", "2.1.278-a/b", " 2.1.278",
		"1.0.0-01", "1.0.0-a..b", "1.0.0-.a", "1.0.0-a_b", "-1.0.0", "1.0.0-é", "1.0.0+", "18446744073709551616.0.0", "1.0.00", "+1.0.0", "1.0.0-a b",
	} {
		if claudeproto.IsVersion(invalid) {
			t.Errorf("%q was accepted", invalid)
		}
	}
}

func TestVersionsAreOrderedBySemVerPrecedence(t *testing.T) {
	ascending := []string{
		"1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0",
		"1.0.1", "1.9.0", "1.10.0", "2.1.9", "2.1.10", "2.1.278-beta.1", "2.1.278", "10.0.0",
	}
	for i, low := range ascending {
		for j, high := range ascending {
			a, _ := claudeproto.ParseVersion(low)
			b, _ := claudeproto.ParseVersion(high)
			want := 0
			if i < j {
				want = -1
			} else if i > j {
				want = 1
			}
			if got := a.Compare(b); got != want {
				t.Errorf("Compare(%s, %s) = %d, want %d", low, high, got, want)
			}
		}
	}
}

func TestRepliesCarryOKBesideTheAnswer(t *testing.T) {
	listed := claudeproto.Listed{OK: true, Platform: "darwin-arm64", Installed: []claudeproto.Installed{{Version: "2.1.3", Path: "/opt/claude"}}}
	printed, err := claudeproto.Encode(listed)
	if err != nil {
		t.Fatal(err)
	}
	const want = `{"ok":true,"platform":"darwin-arm64","installed":[{"version":"2.1.3","path":"/opt/claude"}]}`
	if string(printed) != want {
		t.Errorf("printed %s, want %s", printed, want)
	}
	if back, err := claudeproto.Decode[claudeproto.Reply](printed); err != nil || !reflect.DeepEqual(back, claudeproto.Reply(listed)) {
		t.Errorf("decoded %#v, %v", back, err)
	}
	ensured, err := claudeproto.Encode(claudeproto.Ensured{OK: true, Version: "2.1.3", Path: "/opt/claude"})
	if err != nil || string(ensured) != `{"ok":true,"version":"2.1.3","path":"/opt/claude"}` {
		t.Errorf("an ensured reply: %s, %v", ensured, err)
	}
	failed := claudeproto.Failed{Code: claudeproto.VerificationFailed, Message: "digest differs"}
	printed, err = claudeproto.Encode(failed)
	if err != nil || string(printed) != `{"ok":false,"code":"verification_failed","message":"digest differs"}` {
		t.Errorf("a failed reply: %s, %v", printed, err)
	}
	if back, err := claudeproto.Decode[claudeproto.Reply](printed); err != nil || !reflect.DeepEqual(back, claudeproto.Reply(failed)) {
		t.Errorf("decoded %#v, %v", back, err)
	}
	for name, document := range map[string]string{
		"no ok":                     `{"platform":"x","installed":[]}`,
		"ok that is not the answer": `{"ok":false,"platform":"x","installed":[]}`,
		"ok true with a failure":    `{"ok":true,"code":"install_failed","message":"m"}`,
		"a code that is unknown":    `{"ok":false,"code":"cancelled","message":"m"}`,
		"members of both":           `{"ok":true,"version":"1.0.0","path":"/p","platform":"x","installed":[]}`,
	} {
		if _, err := claudeproto.Decode[claudeproto.Reply]([]byte(document)); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
}

func TestThePackagesOperationsAreItsCatalog(t *testing.T) {
	if got := claudeproto.Operations(); !slices.Equal(got, []string{"claude.ensure", "claude.status"}) || claudeproto.Package != "demi.claude" {
		t.Errorf("operations %v of %s", got, claudeproto.Package)
	}
}

package claudecodeop_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
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
		{"non HTTP URL", "2.1.3", "ftp://example.test/claude", 3, digest, true},
		{"full size range", "2.1.3", "https://example.test/claude", ^uint64(0), digest, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			// A second, valid platform ensures all entries are checked, regardless
			// of which target the eventual caller runs on.
			input := fmt.Sprintf(`{"version":%q,"platforms":{"darwin-arm64":{"url":%q,"size":%d,"sha256":%q},"linux-arm64":{"url":"https://example.test/claude","size":3,"sha256":%q}}}`, tc.version, tc.url, tc.size, tc.digest, digest)
			release, err := claudecodeop.DecodeRelease([]byte(input))
			if (err == nil) != tc.valid {
				t.Fatalf("DecodeRelease() error = %v, want valid %v", err, tc.valid)
			}
			if tc.valid && release.Platforms["darwin-arm64"].Size != tc.size {
				t.Fatalf("size = %d, want %d", release.Platforms["darwin-arm64"].Size, tc.size)
			}
		})
	}
	for _, input := range []string{
		`{"version":"2.1.3","platforms":{},"extra":1}`,
		`{"version":"2.1.3"}`,
		`{"version":"2.1.3","platforms":null}`,
		`{"version":"2.1.3","version":"2.1.4","platforms":{}}`,
	} {
		if _, err := claudecodeop.DecodeRelease([]byte(input)); err == nil {
			t.Errorf("DecodeRelease(%s) succeeded", input)
		}
	}
}

func TestVersionDirectoryNames(t *testing.T) {
	for _, valid := range []string{"1.0.0", "10.20.30-rc.1", "0.0.1-alpha-2", "18446744073709551615.0.0"} {
		if _, err := claudecodeop.DecodeVersion([]byte(fmt.Sprintf("%q", valid))); err != nil {
			t.Errorf("version %q: %v", valid, err)
		}
	}
	for _, invalid := range []string{
		"1.0", "1.0.0-", "1.0.0+build", "v1.0.0", "1..0", "../1.0.0", "01.0.0",
		"1", "1.0.0-01", "18446744073709551616.0.0", "1.18446744073709551616.0", "1.0.18446744073709551616",
	} {
		if _, err := claudecodeop.DecodeVersion([]byte(fmt.Sprintf("%q", invalid))); err == nil {
			t.Errorf("version %q was accepted", invalid)
		}
	}
}

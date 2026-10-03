package claudecode

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"github.com/wspl/demi/internal/commandwire"
)

func releaseRecord(version, platform string) []byte {
	return fmt.Appendf(nil, `{"version":%q,"platforms":{%q:{"url":"https://downloads.claude.ai/claude","size":6,"sha256":"%x"}}}`, version, platform, sha256.Sum256([]byte("claude")))
}

func TestEnsureAsksRunnerForMachineExecutable(t *testing.T) {
	asked := make(chan commandwire.ArtifactRequest, 1)
	var count atomic.Int32
	artifacts := cmdsdktest.ArtifactsFrom(t, func(_ context.Context, request commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
		if count.Add(1) == 1 {
			asked <- request
		}
		path := "/cache/claude"
		return commandwire.ArtifactAnswer{Path: &path}, nil
	})
	release, err := parseRelease(releaseRecord("2.1.278", currentPlatform()))
	if err != nil {
		t.Fatal(err)
	}
	installed, err := ensure(t.Context(), artifacts, "invocation", release)
	if err != nil {
		t.Fatal(err)
	}
	if got, want := installed, (claudecodeop.Installed{Version: "2.1.278", Path: "/cache/claude"}); !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	if count.Load() != 1 {
		t.Fatalf("install requests = %d", count.Load())
	}
	want := commandwire.ArtifactRequest{Install: &commandwire.ArtifactInstall{Invocation: "invocation", Name: "Claude Code", Version: "2.1.278", SHA256: fmt.Sprintf("%x", sha256.Sum256([]byte("claude"))), Size: 6, Form: &commandwire.ArtifactFile{}}}
	if got, want := <-asked, want; !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
}

func TestUnsupportedPlatformAndRunnerFailure(t *testing.T) {
	var asked atomic.Int32
	artifacts := cmdsdktest.ArtifactsFrom(t, func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
		asked.Add(1)
		return commandwire.ArtifactAnswer{}, errors.New("the download failed")
	})
	elsewhere, err := parseRelease(releaseRecord("2.1.278", "other-platform"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = ensure(t.Context(), artifacts, "invocation", elsewhere)
	assertCode(t, err, claudecodeop.UnsupportedPlatform)
	if asked.Load() != 0 {
		t.Fatal("unsupported platform asked runner")
	}
	release, err := parseRelease(releaseRecord("2.1.278", currentPlatform()))
	if err != nil {
		t.Fatal(err)
	}
	_, err = ensure(t.Context(), artifacts, "invocation", release)
	assertCode(t, err, claudecodeop.InstallFailed)
	if !strings.Contains(err.Error(), "the download failed") {
		t.Fatal(err)
	}
}

func TestStatusAnswersRunnerVersions(t *testing.T) {
	artifacts := cmdsdktest.ArtifactsFrom(t, func(_ context.Context, request commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
		if request.Installed == nil || request.Installed.Name != "Claude Code" {
			return commandwire.ArtifactAnswer{}, fmt.Errorf("unexpected request: %+v", request)
		}
		installed := []commandwire.InstalledArtifact{}
		for _, version := range []string{"2.1.278", "2.1.10"} {
			installed = append(installed, commandwire.InstalledArtifact{Version: version, Path: "/cache/" + version, SHA256: fmt.Sprintf("%x", sha256.Sum256([]byte(version)))})
		}
		return commandwire.ArtifactAnswer{Installed: &installed}, nil
	})
	result, err := status(t.Context(), artifacts)
	if err != nil {
		t.Fatal(err)
	}
	want := claudecodeop.Status{Platform: currentPlatform(), Installed: []claudecodeop.Installed{{Version: "2.1.278", Path: "/cache/2.1.278"}, {Version: "2.1.10", Path: "/cache/2.1.10"}}}
	if got, want := result, want; !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
}

func assertCode(t *testing.T, err error, code claudecodeop.ErrorCode) {
	t.Helper()
	var failure *operationError
	if !errors.As(err, &failure) || failure.code != code {
		t.Fatalf("error = %v, want code %s", err, code)
	}
}

func TestMalformedRecordsAreInvalid(t *testing.T) {
	record := string(releaseRecord("2.1.278", "linux-x64"))
	digest := fmt.Sprintf("%x", sha256.Sum256([]byte("claude")))
	invalid := []string{
		`"not a record"`, `{"version":"2.1.278"}`, `{"version":"2.1.278","platforms":{},"extra":true}`,
		strings.Replace(record, `"size":6`, `"extra":1,"size":6`, 1),
		strings.Replace(record, "https://downloads.claude.ai/claude", "http://downloads.claude.ai/claude", 1),
		strings.Replace(record, "https://downloads.claude.ai/claude", "http://127.0.0.1/claude", 1),
		strings.Replace(record, `"size":6`, `"size":0`, 1),
		strings.Replace(record, digest, "abc", 1), strings.Replace(record, digest, strings.ToUpper(digest), 1), "{",
	}
	for i, input := range invalid {
		t.Run(fmt.Sprint(i), func(t *testing.T) {
			_, err := parseRelease([]byte(input))
			assertCode(t, err, claudecodeop.InvalidRelease)
		})
	}
	for _, version := range []string{"", "2.1", "2.1.278.1", "v2.1.278", "2.1.x", "../2.1.278", "2.1.278/..", "2.1.278-", "2.1.278-a/b", "2.1.278+build", " 2.1.278"} {
		t.Run(version, func(t *testing.T) {
			_, err := parseRelease(releaseRecord(version, "linux-x64"))
			assertCode(t, err, claudecodeop.InvalidRelease)
		})
	}
	for _, version := range []string{"2.1.278", "0.0.0", "10.20.30-beta.1", "1.0.0-rc-1"} {
		if _, err := parseRelease(releaseRecord(version, "linux-x64")); err != nil {
			t.Fatalf("%s: %v", version, err)
		}
	}
}

func TestPlatformKeyFollowsMachine(t *testing.T) {
	for _, cpu := range []struct{ arch, key string }{{"amd64", "x64"}, {"arm64", "arm64"}} {
		for _, loaded := range []loaders{{}, {musl: true}, {glibc: true}, {musl: true, glibc: true}} {
			for _, system := range []struct{ os, key string }{{"darwin", "darwin"}, {"windows", "win32"}, {"linux", "linux"}} {
				want := system.key + "-" + cpu.key
				if system.os == "linux" && loaded.musl && !loaded.glibc {
					want += "-musl"
				}
				if got := platformKey(system.os, cpu.arch, loaded); got != want {
					t.Fatalf("%s %s %+v = %q, want %q", system.os, cpu.arch, loaded, got, want)
				}
			}
		}
	}
	for _, pair := range [][2]string{{"darwin", "arm"}, {"linux", "riscv64"}, {"freebsd", "amd64"}} {
		if got := platformKey(pair[0], pair[1], loaders{musl: true}); got != "" {
			t.Fatalf("unsupported %v: %q", pair, got)
		}
	}
}

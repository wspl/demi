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

	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
)

func releaseRecord(version, platform string) []byte {
	return fmt.Appendf(
		nil,
		`{"version":%q,"platforms":{%q:{"url":"https://downloads.claude.ai/claude","size":6,"sha256":"%x"}}}`,
		version,
		platform,
		sha256.Sum256([]byte("claude")),
	)
}

func TestEnsureAsksRunnerForMachineExecutable(t *testing.T) {
	asked := make(chan commandproto.ArtifactRequest, 1)
	var count atomic.Int32
	artifacts := commandsdktest.ArtifactsFrom(
		t,
		func(_ context.Context, request commandproto.ArtifactRequest) (commandproto.ArtifactAnswer, error) {
			if count.Add(1) == 1 {
				asked <- request
			}
			path := "/cache/claude"
			return commandproto.ArtifactAnswer{Path: &path}, nil
		},
	)
	release, err := parseRelease(releaseRecord("2.1.278", currentPlatform()))
	if err != nil {
		t.Fatal(err)
	}
	installed, err := ensure(t.Context(), artifacts, "invocation", release)
	if err != nil {
		t.Fatal(err)
	}
	if got, want := installed, (claudecodeproto.Installed{Version: "2.1.278", Path: "/cache/claude"}); !reflect.DeepEqual(
		got,
		want,
	) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	if count.Load() != 1 {
		t.Fatalf("install requests = %d", count.Load())
	}
	want := commandproto.ArtifactRequest{
		Install: &commandproto.ArtifactInstall{
			Invocation: "invocation",
			Name:       "Claude Code",
			Version:    "2.1.278",
			SHA256:     fmt.Sprintf("%x", sha256.Sum256([]byte("claude"))),
			Size:       6,
			Form:       &commandproto.ArtifactFile{},
		},
	}
	if got, want := <-asked, want; !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
}

func TestUnsupportedPlatformAndRunnerFailure(t *testing.T) {
	var asked atomic.Int32
	artifacts := commandsdktest.ArtifactsFrom(
		t,
		func(context.Context, commandproto.ArtifactRequest) (commandproto.ArtifactAnswer, error) {
			asked.Add(1)
			return commandproto.ArtifactAnswer{}, errors.New("the download failed")
		},
	)
	elsewhere, err := parseRelease(releaseRecord("2.1.278", "other-platform"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = ensure(t.Context(), artifacts, "invocation", elsewhere)
	assertCode(t, err, claudecodeproto.UnsupportedPlatform)
	if asked.Load() != 0 {
		t.Fatal("unsupported platform asked runner")
	}
	release, err := parseRelease(releaseRecord("2.1.278", currentPlatform()))
	if err != nil {
		t.Fatal(err)
	}
	_, err = ensure(t.Context(), artifacts, "invocation", release)
	assertCode(t, err, claudecodeproto.InstallFailed)
	if !strings.Contains(err.Error(), "the download failed") {
		t.Fatal(err)
	}
}

func assertCode(t *testing.T, err error, code claudecodeproto.ErrorCode) {
	t.Helper()
	var failure *operationError
	if !errors.As(err, &failure) || failure.code != code {
		t.Fatalf("error = %v, want code %s", err, code)
	}
}

// The contract's own checks are tested in claudecodeop; this adds the HTTPS
// rule parseRelease enforces and the code every refusal carries.
func TestMalformedRecordsAreInvalid(t *testing.T) {
	record := string(releaseRecord("2.1.278", "linux-x64"))
	for _, input := range []string{
		"{",
		string(releaseRecord("../2.1.278", "linux-x64")),
		strings.Replace(record, "https://downloads.claude.ai/claude", "http://downloads.claude.ai/claude", 1),
		strings.Replace(record, "https://downloads.claude.ai/claude", "ftp://127.0.0.1/claude", 1),
	} {
		t.Run(input, func(t *testing.T) {
			_, err := parseRelease([]byte(input))
			assertCode(t, err, claudecodeproto.InvalidRelease)
		})
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

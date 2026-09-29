package claude_test

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/v2"
	"errors"
	"os"
	"os/user"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/artifact/artifacttest"
	"github.com/wspl/demi/go/claude"
	"github.com/wspl/demi/go/claudeproto"
)

// body is a fixture executable.
var body = []byte("#!/bin/sh\necho fixture claude\n")

// versions are the versions whose executable the download server serves.
var versions = []string{"2.1.277", "2.1.278", "2.1.279"}

// binary is the executable's file name inside a version directory.
func binary() string {
	if runtime.GOOS == "windows" {
		return "claude.exe"
	}
	return "claude"
}

func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

func platform(t testing.TB) string {
	t.Helper()
	key, ok := claude.CurrentPlatform()
	if !ok {
		t.Skip("Claude Code has no build for this machine")
	}
	return key
}

// downloads starts a download server for body as each of versions, and holds each
// answer until hold is closed when it is not nil.
func downloads(t *testing.T, hold <-chan struct{}) *artifacttest.Server {
	t.Helper()
	answers := map[string]artifacttest.Answer{}
	for _, version := range versions {
		answer := artifacttest.OK(body)
		answer.Hold = hold
		answers["/"+version+"/claude"] = answer
	}
	return artifacttest.Start(t, answers)
}

// served returns a release record whose entry for this machine points at
// server's download of version.
func served(t testing.TB, server *artifacttest.Server, version string, size int, sha256 string) []byte {
	return record(t, version, platform(t), server.URL("/"+version+"/claude"), size, sha256)
}

func record(t testing.TB, version, platform, url string, size int, sha256 string) []byte {
	t.Helper()
	data, err := json.Marshal(map[string]any{
		"version":   version,
		"platforms": map[string]any{platform: map[string]any{"url": url, "size": size, "sha256": sha256}},
	})
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// A machine is a temporary home and image root.
type machine struct {
	home, image string
}

func newMachine(t *testing.T) machine {
	t.Helper()
	directory := t.TempDir()
	return machine{home: filepath.Join(directory, "home", ".demi", "claude"), image: filepath.Join(directory, "opt", "demi", "claude")}
}

func (m machine) installer() *claude.Installer {
	return claude.NewLoopbackInstaller(claude.Roots{Home: m.home, Image: m.image})
}

// homeDirectories returns the version directories and staging directories left
// under the user's root.
func (m machine) homeDirectories(t *testing.T) []string {
	t.Helper()
	entries, err := os.ReadDir(m.home)
	if err != nil {
		return nil
	}
	var names []string
	for _, entry := range entries {
		if entry.IsDir() {
			names = append(names, entry.Name())
		}
	}
	slices.Sort(names)
	return names
}

func preinstall(t *testing.T, root, version string, data []byte) {
	t.Helper()
	directory := filepath.Join(root, version)
	if err := os.MkdirAll(directory, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, binary()), data, 0o755); err != nil {
		t.Fatal(err)
	}
	receipt, err := json.Marshal(map[string]any{"version": version, "platform": platform(t), "sha256": sha256Hex(data), "size": len(data)})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, "receipt.json"), receipt, 0o644); err != nil {
		t.Fatal(err)
	}
}

func ensure(ctx context.Context, installer *claude.Installer, record []byte) (claudeproto.Installed, error) {
	release, err := installer.Release(record)
	if err != nil {
		return claudeproto.Installed{}, err
	}
	return installer.Ensure(ctx, release)
}

// codeOf returns the code of a failure.
func codeOf(err error) claudeproto.ErrorCode {
	var failure *claude.EnsureError
	if errors.As(err, &failure) {
		return failure.Code
	}
	return ""
}

func TestEnsureInstallsTheExecutableAndItsReceipt(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	installed, err := ensure(t.Context(), m.installer(), served(t, server, "2.1.278", len(body), sha256Hex(body)))
	if err != nil {
		t.Fatal(err)
	}
	if installed.Version != "2.1.278" || installed.Path != filepath.Join(m.home, "2.1.278", binary()) || !filepath.IsAbs(installed.Path) {
		t.Errorf("installed %+v", installed)
	}
	if got, _ := os.ReadFile(installed.Path); string(got) != string(body) {
		t.Errorf("the executable holds %q", got)
	}
	if runtime.GOOS != "windows" {
		if info, err := os.Stat(installed.Path); err != nil || info.Mode()&0o111 != 0o111 {
			t.Errorf("the executable has mode %v, %v", info.Mode(), err)
		}
	}
	data, err := os.ReadFile(filepath.Join(m.home, "2.1.278", "receipt.json"))
	if err != nil {
		t.Fatal(err)
	}
	var receipt map[string]any
	if err := json.Unmarshal(data, &receipt); err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"version": "2.1.278", "platform": platform(t), "sha256": sha256Hex(body), "size": float64(len(body))}
	if !mapsEqual(receipt, want) {
		t.Errorf("receipt %v, want %v", receipt, want)
	}
	if got := m.homeDirectories(t); !slices.Equal(got, []string{"2.1.278"}) {
		t.Errorf("the user's root holds %v", got)
	}
}

func mapsEqual(a, b map[string]any) bool {
	if len(a) != len(b) {
		return false
	}
	for key, value := range a {
		if b[key] != value {
			return false
		}
	}
	return true
}

func TestASecondEnsureDoesNotDownload(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := served(t, server, "2.1.278", len(body), sha256Hex(body))
	installer := m.installer()
	first, err := ensure(t.Context(), installer, record)
	if err != nil {
		t.Fatal(err)
	}
	second, err := ensure(t.Context(), installer, record)
	if err != nil {
		t.Fatal(err)
	}
	// Another service process verifies the installation rather than trusting
	// memory.
	third, err := ensure(t.Context(), m.installer(), record)
	if err != nil {
		t.Fatal(err)
	}
	if first != second || first != third {
		t.Errorf("the three answers differ: %+v %+v %+v", first, second, third)
	}
	if server.Requests() != 1 {
		t.Errorf("the server was asked %d times", server.Requests())
	}
}

// A process hashes an executable once; later calls compare the receipt and the
// size, so an executable that changed in place with the same size is found by
// the next process, not by the same one.
func TestAnExecutableIsHashedOncePerProcess(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := served(t, server, "2.1.278", len(body), sha256Hex(body))
	installer := m.installer()
	installed, err := ensure(t.Context(), installer, record)
	if err != nil {
		t.Fatal(err)
	}
	changed := slices.Clone(body)
	changed[0] ^= 1
	if err := os.WriteFile(installed.Path, changed, 0o755); err != nil {
		t.Fatal(err)
	}
	if _, err := ensure(t.Context(), installer, record); err != nil || server.Requests() != 1 {
		t.Errorf("the same process: %v after %d requests", err, server.Requests())
	}
	if _, err := ensure(t.Context(), m.installer(), record); err != nil || server.Requests() != 2 {
		t.Errorf("another process: %v after %d requests", err, server.Requests())
	}
}

func TestAChangedInstallationIsReplaced(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := served(t, server, "2.1.278", len(body), sha256Hex(body))
	installed, err := ensure(t.Context(), m.installer(), record)
	if err != nil {
		t.Fatal(err)
	}
	changed := slices.Clone(body)
	changed[0] ^= 1
	if err := os.WriteFile(installed.Path, changed, 0o755); err != nil {
		t.Fatal(err)
	}
	if _, err := ensure(t.Context(), m.installer(), record); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(installed.Path); string(got) != string(body) {
		t.Errorf("the executable holds %q", got)
	}
	if server.Requests() != 2 {
		t.Errorf("the server was asked %d times", server.Requests())
	}
}

func TestADownloadThatIsNotTheDeclaredExecutableInstallsNothing(t *testing.T) {
	for name, test := range map[string]struct {
		declared []byte
		size     int
		digest   string
	}{
		"a wrong digest":               {body, len(body), sha256Hex([]byte("another executable"))},
		"a body longer than its size":  {body[:len(body)-1], len(body) - 1, sha256Hex(body[:len(body)-1])},
		"a body shorter than its size": {body, len(body) + 1, sha256Hex(body)},
	} {
		m := newMachine(t)
		server := downloads(t, nil)
		record := served(t, server, "2.1.278", test.size, test.digest)
		_, err := ensure(t.Context(), m.installer(), record)
		if codeOf(err) != claudeproto.VerificationFailed {
			t.Errorf("%s: %v", name, err)
		}
		if name == "a wrong digest" && (!strings.Contains(err.Error(), "2.1.278") || !strings.Contains(err.Error(), "127.0.0.1")) {
			t.Errorf("%s: the message names neither the version nor the host: %v", name, err)
		}
		if got := m.homeDirectories(t); len(got) != 0 {
			t.Errorf("%s: the user's root holds %v", name, got)
		}
	}
}

func TestADownloadTheServerRefusesIsADownloadFailure(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := record(t, "2.1.278", platform(t), server.URL("/elsewhere"), len(body), sha256Hex(body))
	_, err := ensure(t.Context(), m.installer(), record)
	if codeOf(err) != claudeproto.DownloadFailed || !strings.Contains(err.Error(), "the server answered 404") {
		t.Errorf("a missing artifact: %v", err)
	}
	if got := m.homeDirectories(t); len(got) != 0 {
		t.Errorf("the user's root holds %v", got)
	}
}

func TestARecordWithoutThisPlatformIsUnsupported(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := record(t, "2.1.278", "plan9-mips", server.URL("/claude"), len(body), sha256Hex(body))
	_, err := ensure(t.Context(), m.installer(), record)
	if codeOf(err) != claudeproto.UnsupportedPlatform || !strings.Contains(err.Error(), platform(t)) {
		t.Errorf("a record without this platform: %v", err)
	}
	if server.Requests() != 0 {
		t.Errorf("the server was asked %d times", server.Requests())
	}
}

// Only versions are removed: a directory that is not one may be another
// installer's staging, which its owner still needs.
func TestANewVersionRemovesOtherVersionsAndNothingElse(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	for _, name := range []string{"2.1.277", "2.1.276-beta.1", "install-1234", "notes", "2.1"} {
		if err := os.MkdirAll(filepath.Join(m.home, name), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(m.home, "2.1.275"), []byte("a file that is named as a version"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := ensure(t.Context(), m.installer(), served(t, server, "2.1.278", len(body), sha256Hex(body))); err != nil {
		t.Fatal(err)
	}
	want := []string{"2.1", "2.1.278", "install-1234", "notes"}
	if got := m.homeDirectories(t); !slices.Equal(got, want) {
		t.Errorf("the user's root holds %v, want %v", got, want)
	}
	if _, err := os.Stat(filepath.Join(m.home, "2.1.275")); err != nil {
		t.Errorf("a file named as a version was removed: %v", err)
	}
}

// An installation is used only when its receipt is the one the record implies
// and its executable has the declared size, even when the process has hashed
// the executable already.
func TestAnInstallationThatIsNotWhatTheRecordDeclaresIsNotUsed(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := served(t, server, "2.1.278", len(body), sha256Hex(body))
	installer := m.installer()
	installed, err := ensure(t.Context(), installer, record)
	if err != nil {
		t.Fatal(err)
	}
	// The process has hashed the executable, so only its size is looked at now.
	if err := os.WriteFile(installed.Path, body[:len(body)-1], 0o755); err != nil {
		t.Fatal(err)
	}
	if _, err := ensure(t.Context(), installer, record); err != nil || server.Requests() != 2 {
		t.Errorf("a truncated executable: %v after %d requests", err, server.Requests())
	}
	if got, _ := os.ReadFile(installed.Path); string(got) != string(body) {
		t.Errorf("the executable holds %q", got)
	}
	// A receipt that names other bytes, though the executable is right, is not
	// the one the record implies: the image's installation is ignored.
	for name, receipt := range map[string]map[string]any{
		"another digest":   {"version": "2.1.278", "platform": platform(t), "sha256": sha256Hex([]byte("other")), "size": len(body)},
		"another size":     {"version": "2.1.278", "platform": platform(t), "sha256": sha256Hex(body), "size": len(body) + 1},
		"another platform": {"version": "2.1.278", "platform": "plan9-mips", "sha256": sha256Hex(body), "size": len(body)},
		"another version":  {"version": "2.1.277", "platform": platform(t), "sha256": sha256Hex(body), "size": len(body)},
	} {
		machine := newMachine(t)
		preinstall(t, machine.image, "2.1.278", body)
		data, err := json.Marshal(receipt)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(machine.image, "2.1.278", "receipt.json"), data, 0o644); err != nil {
			t.Fatal(err)
		}
		used, err := ensure(t.Context(), machine.installer(), record)
		if err != nil || used.Path != filepath.Join(machine.home, "2.1.278", binary()) {
			t.Errorf("%s: %+v, %v", name, used, err)
		}
	}
}

func TestANewVersionRemovesTheOldOne(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	installer := m.installer()
	for _, version := range []string{"2.1.277", "2.1.278"} {
		if _, err := ensure(t.Context(), installer, served(t, server, version, len(body), sha256Hex(body))); err != nil {
			t.Fatal(err)
		}
	}
	if got := m.homeDirectories(t); !slices.Equal(got, []string{"2.1.278"}) {
		t.Errorf("the user's root holds %v", got)
	}
}

func TestAPreinstalledVersionIsUsedWithoutDownloading(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	preinstall(t, m.image, "2.1.278", body)
	installed, err := ensure(t.Context(), m.installer(), served(t, server, "2.1.278", len(body), sha256Hex(body)))
	if err != nil {
		t.Fatal(err)
	}
	if installed.Path != filepath.Join(m.image, "2.1.278", binary()) || server.Requests() != 0 {
		t.Errorf("installed %+v after %d requests", installed, server.Requests())
	}
	if got := m.homeDirectories(t); len(got) != 0 {
		t.Errorf("the user's root holds %v", got)
	}
}

func TestAMismatchingPreinstalledVersionIsIgnoredAndKept(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	older := []byte("an older build of the same version")
	preinstall(t, m.image, "2.1.278", older)
	installed, err := ensure(t.Context(), m.installer(), served(t, server, "2.1.278", len(body), sha256Hex(body)))
	if err != nil {
		t.Fatal(err)
	}
	if installed.Path != filepath.Join(m.home, "2.1.278", binary()) {
		t.Errorf("installed %+v", installed)
	}
	if got, _ := os.ReadFile(filepath.Join(m.image, "2.1.278", binary())); string(got) != string(older) {
		t.Errorf("the image's executable holds %q", got)
	}
}

// Three ensures of one version at once, two in one installer as two invocations
// of a service and one in another as another process, wait for an installer that
// holds the version's lock; once it lets go, one of them downloads and the others
// find its installation.
func TestConcurrentEnsuresDownloadOnce(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	record := served(t, server, "2.1.278", len(body), sha256Hex(body))
	if err := os.MkdirAll(m.home, 0o755); err != nil {
		t.Fatal(err)
	}
	var waits artifacttest.LockWaits
	ctx := waits.Watch(t.Context())
	held, err := artifact.AcquireInstallLock(ctx, filepath.Join(m.home, "2.1.278.lock"))
	if err != nil {
		t.Fatal(err)
	}
	// The held lock was taken without a wait.
	if waits.Count() != 0 {
		t.Fatalf("%d waits before anyone waited", waits.Count())
	}
	service, process := m.installer(), m.installer()
	type outcome struct {
		installed claudeproto.Installed
		err       error
	}
	results := make(chan outcome, 3)
	for _, installer := range []*claude.Installer{service, service, process} {
		go func() {
			installed, err := ensure(ctx, installer, record)
			results <- outcome{installed, err}
		}()
	}
	for range 3 {
		<-waits.Waited()
	}
	held.Close()
	var first claudeproto.Installed
	for i := range 3 {
		result := <-results
		if result.err != nil {
			t.Fatal(result.err)
		}
		if i == 0 {
			first = result.installed
		} else if result.installed != first {
			t.Errorf("answers differ: %+v and %+v", first, result.installed)
		}
	}
	if server.Requests() != 1 {
		t.Errorf("the server was asked %d times", server.Requests())
	}
	if got := m.homeDirectories(t); !slices.Equal(got, []string{"2.1.278"}) {
		t.Errorf("the user's root holds %v", got)
	}
}

// The server holds its answer: the download is under way when the test cancels it.
func TestCancellationStopsADownloadAndInstallsNothing(t *testing.T) {
	m := newMachine(t)
	hold := make(chan struct{})
	defer close(hold)
	server := downloads(t, hold)
	installer := m.installer()
	release, err := installer.Release(served(t, server, "2.1.278", len(body), sha256Hex(body)))
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() {
		_, err := installer.Ensure(ctx, release)
		done <- err
	}()
	<-server.Arrived()
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled ensure: %v", err)
	}
	if got := m.homeDirectories(t); len(got) != 0 {
		t.Errorf("the user's root holds %v", got)
	}
}

func TestMalformedRecordsAreInvalid(t *testing.T) {
	installer := claude.NewInstaller(claude.Roots{})
	digest := sha256Hex(body)
	artifactOf := func(url string, size int, sha256 string, extra ...string) string {
		member := ""
		if len(extra) > 0 {
			member = `,"extra":1`
		}
		data, _ := json.Marshal(map[string]any{"url": url, "size": size, "sha256": sha256})
		return strings.TrimSuffix(string(data), "}") + member + "}"
	}
	release := func(version, artifact string) string {
		return `{"version":"` + version + `","platforms":{"linux-x64":` + artifact + `}}`
	}
	const https = "https://downloads.claude.ai/claude"
	for name, invalid := range map[string]string{
		"not a record":                `"not a record"`,
		"no platforms":                `{"version":"2.1.278"}`,
		"a member more":               `{"version":"2.1.278","platforms":{},"extra":true}`,
		"an entry with a member more": release("2.1.278", artifactOf(https, 1, digest, "extra")),
		"plain HTTP":                  release("2.1.278", artifactOf("http://downloads.claude.ai/claude", 1, digest)),
		"loopback HTTP, which the program refuses": release("2.1.278", artifactOf("http://127.0.0.1/claude", 1, digest)),
		"a size of nothing":                        release("2.1.278", artifactOf(https, 0, digest)),
		"a short digest":                           release("2.1.278", artifactOf(https, 1, "abc")),
		"a digest in capitals":                     release("2.1.278", artifactOf(https, 1, strings.ToUpper(digest))),
		"not JSON":                                 `{`,
	} {
		if _, err := installer.Release([]byte(invalid)); codeOf(err) != claudeproto.InvalidRelease {
			t.Errorf("%s: %v", name, err)
		}
	}
	for _, version := range []string{"", "2.1", "2.1.278.1", "v2.1.278", "2.1.x", "../2.1.278", "2.1.278/..", "2.1.278-", "2.1.278-a/b", "2.1.278+build", " 2.1.278"} {
		if _, err := installer.Release([]byte(release(version, artifactOf(https, 1, digest)))); codeOf(err) != claudeproto.InvalidRelease {
			t.Errorf("version %q: %v", version, err)
		}
	}
	for _, version := range []string{"2.1.278", "0.0.0", "10.20.30-beta.1", "1.0.0-rc-1"} {
		if _, err := installer.Release([]byte(release(version, artifactOf(https, 1, digest)))); err != nil {
			t.Errorf("version %q: %v", version, err)
		}
	}
	// Every platform's URL is checked, not only this machine's.
	other := `{"version":"2.1.278","platforms":{"linux-x64":` + artifactOf(https, 1, digest) + `,"win32-x64":` + artifactOf("ftp://downloads.claude.ai/claude", 1, digest) + `}}`
	if _, err := installer.Release([]byte(other)); codeOf(err) != claudeproto.InvalidRelease || !strings.Contains(err.Error(), "platform win32-x64: url must be https") {
		t.Errorf("a platform that is not this machine's: %v", err)
	}
}

func TestThePlatformKeyFollowsTheMachine(t *testing.T) {
	none := claude.Loaders{}
	glibc := claude.Loaders{Glibc: true}
	musl := claude.Loaders{Musl: true}
	both := claude.Loaders{Musl: true, Glibc: true}
	for _, test := range []struct {
		goos, goarch string
		loaders      claude.Loaders
		want         string
	}{
		{"darwin", "arm64", none, "darwin-arm64"},
		{"darwin", "amd64", none, "darwin-x64"},
		{"windows", "amd64", none, "win32-x64"},
		{"windows", "arm64", none, "win32-arm64"},
		{"linux", "amd64", glibc, "linux-x64"},
		{"linux", "amd64", musl, "linux-x64-musl"},
		{"linux", "amd64", both, "linux-x64"},
		{"linux", "amd64", none, "linux-x64"},
		{"linux", "arm64", glibc, "linux-arm64"},
		{"linux", "arm64", musl, "linux-arm64-musl"},
		{"linux", "arm64", both, "linux-arm64"},
		{"linux", "arm64", none, "linux-arm64"},
		{"darwin", "arm", musl, ""},
		{"linux", "riscv64", glibc, ""},
		{"freebsd", "amd64", none, ""},
	} {
		if got, ok := claude.PlatformKey(test.goos, test.goarch, test.loaders); got != test.want || ok != (test.want != "") {
			t.Errorf("PlatformKey(%s, %s, %+v) = %q, %v; want %q", test.goos, test.goarch, test.loaders, got, ok, test.want)
		}
	}
}

func TestStatusListsInstallationsNewestFirst(t *testing.T) {
	m := newMachine(t)
	preinstall(t, m.home, "2.1.9", body)
	preinstall(t, m.home, "2.1.278", body)
	preinstall(t, m.home, "2.1.278-beta.1", body)
	preinstall(t, m.image, "2.1.10", body)
	// Not installations: no receipt, no executable, a receipt of another version,
	// a stray name.
	if err := os.MkdirAll(filepath.Join(m.home, "3.0.0"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(m.home, "3.0.0", binary()), body, 0o755); err != nil {
		t.Fatal(err)
	}
	preinstall(t, m.home, "3.0.1", body)
	if err := os.Remove(filepath.Join(m.home, "3.0.1", binary())); err != nil {
		t.Fatal(err)
	}
	preinstall(t, m.home, "3.0.2", body)
	if err := os.Rename(filepath.Join(m.home, "3.0.2"), filepath.Join(m.home, "3.0.3")); err != nil {
		t.Fatal(err)
	}
	preinstall(t, m.home, "install-abc", body)
	if err := os.WriteFile(filepath.Join(m.home, "2.1.278.lock"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	status, err := m.installer().Status()
	if err != nil {
		t.Fatal(err)
	}
	if status.Platform != platform(t) {
		t.Errorf("platform %q", status.Platform)
	}
	want := []claudeproto.Installed{
		{Version: "2.1.278", Path: filepath.Join(m.home, "2.1.278", binary())},
		{Version: "2.1.278-beta.1", Path: filepath.Join(m.home, "2.1.278-beta.1", binary())},
		{Version: "2.1.10", Path: filepath.Join(m.image, "2.1.10", binary())},
		{Version: "2.1.9", Path: filepath.Join(m.home, "2.1.9", binary())},
	}
	if !slices.Equal(status.Installed, want) {
		t.Errorf("installed %+v, want %+v", status.Installed, want)
	}
}

func TestAMachineWithoutAHomeDirectoryCannotInstall(t *testing.T) {
	server := downloads(t, nil)
	for name, home := range map[string]string{"no home": "", "a relative home": "relative/home"} {
		installer := claude.NewLoopbackInstaller(claude.Roots{Home: home})
		_, err := ensure(t.Context(), installer, served(t, server, "2.1.278", len(body), sha256Hex(body)))
		if codeOf(err) != claudeproto.InstallFailed {
			t.Errorf("%s: %v", name, err)
		}
	}
	if server.Requests() != 0 {
		t.Errorf("the server was asked %d times", server.Requests())
	}
}

func TestTheDefaultHomeIsTheAccountsWhenTheEnvironmentNamesNone(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows has no HOME")
	}
	account, err := user.Current()
	if err != nil || account.HomeDir == "" {
		t.Skip("the account has no home directory")
	}
	t.Setenv("HOME", "")
	if got, want := claude.DefaultRoots().Home, filepath.Join(account.HomeDir, ".demi", "claude"); got != want {
		t.Errorf("home %q, want %q", got, want)
	}
	t.Setenv("HOME", "/somewhere")
	if got, want := claude.DefaultRoots().Home, filepath.Join("/somewhere", ".demi", "claude"); got != want {
		t.Errorf("home %q, want %q", got, want)
	}
}

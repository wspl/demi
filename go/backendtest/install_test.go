package backendtest_test

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// runnerTargets are the six platforms a runner release names, and the versions
// its manifest carries: the runner wire's and the command protocol's.
var runnerTargets = []string{
	"aarch64-apple-darwin", "x86_64-apple-darwin", "aarch64-unknown-linux-musl",
	"x86_64-unknown-linux-musl", "aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc",
}

const commandProtocolVersion = 1

func sha(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

// A runnerReleases is a runner release directory whose releases all carry one
// program.
type runnerReleases struct {
	t       *testing.T
	dir     string
	program string
	// artifact is the program's size and digest, which every release names.
	artifact map[string]any
}

func newRunnerReleases(t *testing.T, program string) *runnerReleases {
	t.Helper()
	content, err := os.ReadFile(program)
	if err != nil {
		t.Fatal(err)
	}
	dir, err := os.MkdirTemp("", "demi-releases-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	return &runnerReleases{
		t: t, dir: dir, program: program,
		artifact: map[string]any{"sha256": sha(content), "size": len(content)},
	}
}

// publish publishes a release named by name's digest, which the top-level
// manifest names from now on. The release links the program rather than copying
// it.
func (r *runnerReleases) publish(name string) string {
	r.t.Helper()
	release := sha([]byte(name))
	target := string(backendtest.HostTarget(r.t))
	executable := filepath.Join(r.dir, release, target, "demi-runner")
	if err := os.MkdirAll(filepath.Dir(executable), 0o755); err != nil {
		r.t.Fatal(err)
	}
	if err := os.Symlink(r.program, executable); err != nil {
		r.t.Fatal(err)
	}
	// Test-only: every target names this machine's runner.
	targets := map[string]any{}
	for _, name := range runnerTargets {
		targets[name] = r.artifact
	}
	manifest := backendtest.Marshal(map[string]any{
		"release": release, "wire": backendtest.RunnerVersion, "commandProtocol": commandProtocolVersion, "targets": targets,
	})
	for _, path := range []string{filepath.Join(r.dir, release, "manifest.json"), filepath.Join(r.dir, "manifest.json")} {
		if err := os.WriteFile(path, manifest, 0o644); err != nil {
			r.t.Fatal(err)
		}
	}
	return release
}

// installations are the installations a test made under home, each drained when
// the test ends, however it ends.
type installations struct {
	t      *testing.T
	home   string
	mu     sync.Mutex
	states []string
}

func newInstallations(t *testing.T) *installations {
	t.Helper()
	home, err := os.MkdirTemp("", "demi-install-home-")
	if err != nil {
		t.Fatal(err)
	}
	i := &installations{t: t, home: home}
	t.Cleanup(func() {
		i.drain()
		_ = os.RemoveAll(home)
	})
	return i
}

// state is the installation state of the backend at url.
func (i *installations) state(url string) string {
	state := filepath.Join(i.home, ".demi/instances", sha([]byte(url)))
	i.mu.Lock()
	defer i.mu.Unlock()
	i.states = append(i.states, state)
	return state
}

// drain drains the runner of every installation.
func (i *installations) drain() {
	i.mu.Lock()
	defer i.mu.Unlock()
	for _, state := range i.states {
		launcher := filepath.Join(state, "run")
		if _, err := os.Stat(launcher); err == nil {
			// A runner that never started has nothing to drain.
			_ = exec.Command(launcher, "drain").Run()
		}
	}
	i.states = nil
}

// shell is a shell for an installer, with the home and without an installation
// ID.
func (i *installations) shell(extra map[string]string, arguments ...string) *exec.Cmd {
	command := exec.Command("sh", arguments...)
	var env []string
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "HOME=") && !strings.HasPrefix(entry, "DEMI_INSTALLATION_ID=") {
			env = append(env, entry)
		}
	}
	env = append(env, "HOME="+i.home)
	for name, value := range extra {
		env = append(env, name+"="+value)
	}
	command.Env = env
	return command
}

// script saves the backend's installer in the home.
func (i *installations) script(b *backendtest.Backend) string {
	i.t.Helper()
	script := b.Get("/install.sh", nil).Expect(http.StatusOK)
	path := filepath.Join(i.home, fmt.Sprintf("install-%d.sh", b.Port))
	if err := os.WriteFile(path, script.Body, 0o644); err != nil {
		i.t.Fatal(err)
	}
	return path
}

// install fetches the backend's installer and runs it.
func (i *installations) install(b *backendtest.Backend) (string, error) {
	i.t.Helper()
	return i.run(i.script(b), nil)
}

func (i *installations) run(script string, extra map[string]string) (string, error) {
	output, err := i.shell(extra, script).CombinedOutput()
	return string(output), err
}

// succeeded fails the test unless the installer succeeded, and answers its
// output.
func succeeded(t *testing.T, output string, err error) string {
	t.Helper()
	if err != nil {
		t.Fatalf("%v\n%s", err, output)
	}
	return output
}

// active is the installation's active runner: its endpoint and its release.
func active(t *testing.T, state string) map[string]any {
	t.Helper()
	content, err := os.ReadFile(filepath.Join(state, "active.json"))
	if err != nil {
		t.Fatal(err)
	}
	return backendtest.Decode(t, content).(map[string]any)
}

func modeOf(t *testing.T, path string) os.FileMode {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info.Mode().Perm()
}

// Cost: two backends and the installer run for real, several seconds: each of
// its three installs downloads this build's runner (170 MB) from its backend,
// verifies it and starts it; the upgrade drains one.
func TestAnInstallerKeepsEachBackendApartReusesAReleaseAndUpgradesOnlyItsOwnRunner(t *testing.T) {
	t.Parallel()
	releases := newRunnerReleases(t, backendtest.Program(t, "demi-runner"))
	initial := releases.publish("initial")
	a := backendtest.New(t, backendtest.WithRunnerReleases(releases.dir)).Start()
	b := backendtest.New(t, backendtest.WithRunnerReleases(releases.dir)).Start()
	installed := newInstallations(t)
	stateA := installed.state(a.URL + "/")
	stateB := installed.state(b.URL + "/")

	// The Windows installer names the release's Windows runners.
	windows := a.Get("/install.ps1", nil).Expect(http.StatusOK)
	headerIs(t, windows, "Content-Type", "text/plain; charset=utf-8")
	contains(t, windows.Text(), "aarch64-pc-windows-msvc")

	output, err := installed.install(a)
	succeeded(t, output, err)
	output, err = installed.install(b)
	succeeded(t, output, err)
	firstA, firstB := active(t, stateA), active(t, stateB)
	if firstA["endpoint"] == firstB["endpoint"] {
		t.Fatal("two backends share a runner")
	}
	if firstA["release"] != initial {
		t.Fatalf("the release is %v, not %s", firstA["release"], initial)
	}

	// Installed again, the running runner stays.
	output, err = installed.install(a)
	contains(t, succeeded(t, output, err), "already running")
	if active(t, stateA)["endpoint"] != firstA["endpoint"] {
		t.Fatal("a second install replaced the running runner")
	}

	// One backend's installer cannot take another backend's installation.
	scriptB := filepath.Join(installed.home, fmt.Sprintf("install-%d.sh", b.Port))
	registrationA := sha([]byte(a.URL + "/"))
	output, err = installed.run(scriptB, map[string]string{"DEMI_INSTALLATION_ID": registrationA})
	exitError, ok := err.(*exec.ExitError)
	if !ok || exitError.ExitCode() != 1 {
		t.Fatalf("the collision ends with %v", err)
	}
	contains(t, output, "another backend")

	// A new release drains and replaces this backend's runner alone, and the old
	// release's runner stays downloadable.
	upgraded := releases.publish("upgraded")
	output, err = installed.install(a)
	succeeded(t, output, err)
	old := a.Do(backendtest.Request{Method: http.MethodHead, Path: "/runner-artifacts/" + initial + "/" + string(backendtest.HostTarget(t)) + "/demi-runner"})
	old.Expect(http.StatusOK)
	headerIs(t, old, "Cache-Control", "public, max-age=31536000, immutable")
	upgradedA := active(t, stateA)
	if upgradedA["release"] != upgraded || upgradedA["endpoint"] == firstA["endpoint"] {
		t.Fatalf("the upgrade left %v", upgradedA)
	}
	if active(t, stateB)["endpoint"] != firstB["endpoint"] {
		t.Fatal("the upgrade replaced the other backend's runner")
	}
	installed.drain()
	a.Stop()
	b.Stop()
}

// Cost: one backend, the installer run for real and a scripted vendor, several
// seconds (8 s under load): the installer downloads this build's runner (170 MB)
// from its backend, verifies it and starts it; a scripted model then runs one
// job on the paired runner.
func TestAnInstalledRunnerWorksWithTheMaskOfTheShellThatRanTheInstaller(t *testing.T) {
	t.Parallel()
	releases := newRunnerReleases(t, backendtest.Program(t, "demi-runner"))
	releases.publish("initial")
	b, master := backendtest.New(t, backendtest.WithRunnerReleases(releases.dir)).StartSetUp()
	installed := newInstallations(t)
	state := installed.state(b.URL + "/")

	// The user's shell lets the group write, as some systems' shells do.
	script := installed.script(b)
	command := installed.shell(nil, "-c", `umask 002 && exec sh "$0"`, script)
	output, err := command.CombinedOutput()
	succeeded(t, string(output), err)
	// The installation stays the user's alone: its log holds the pairing code.
	if got := modeOf(t, state); got != 0o700 {
		t.Fatalf("the installation's mode is %o", got)
	}
	if got := modeOf(t, filepath.Join(state, "runner.log")); got != 0o600 {
		t.Fatalf("the runner's log's mode is %o", got)
	}

	log := filepath.Join(state, "runner.log")
	var code string
	backendtest.Eventually(t, "the installed runner prints its pairing code", func() bool {
		content, _ := os.ReadFile(log)
		for line := range strings.SplitSeq(string(content), "\n") {
			if rest, ok := strings.CutPrefix(line, "demi-runner: pairing code: "); ok {
				code = strings.TrimSpace(rest)
			}
		}
		return code != ""
	})
	claimed := b.Post("/api/devices/claim", master, backendtest.Map{"code": code}).Expect(http.StatusCreated)
	device := claimed.Str("device.id")
	b.UntilOnline(master, device, true)

	// The agent's job reports the user's mask, and a file it makes has the mode
	// the user's own programs would give it.
	vendor := scripted.StartVendor(t)
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{
		"target": backendtest.Map{"kind": "device", "deviceId": device, "path": installed.home},
	}).Expect(http.StatusOK)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.ToolUse("mask", "shell_exec", backendtest.Map{"script": "umask; echo made > made.txt", "timeoutMs": 60_000}))
	vendor.Respond(scripted.Answer([]string{"done"}, 1, 1))
	socket.Chat("m1", "show the mask")
	result := scripted.ToolResult(t, scenarioItem(t, vendor.Requests(), 1).JSON(t), "mask")
	contains(t, result, "0002")
	if got := modeOf(t, filepath.Join(installed.home, "made.txt")); got != 0o664 {
		t.Fatalf("the file's mode is %o", got)
	}
	installed.drain()
	b.Stop()
}

// Cost: two backends, about a second.
func TestWithoutRunnerReleasesTheInstallersSaySoAndNoArtifactIsServed(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t).Start()
	script := b.Get("/install.sh", nil).Expect(http.StatusServiceUnavailable)
	if script.Text() != "Runner releases are not configured on this backend.\n" {
		t.Fatalf("the installer says %q", script.Text())
	}
	target := string(backendtest.HostTarget(t))
	b.Get("/runner-artifacts/"+sha([]byte("initial"))+"/"+target+"/demi-runner", nil).Expect(http.StatusNotFound)

	// With them, only a release's own executable is served, by its name. No
	// installer runs here, so a stand-in carries each release.
	standIn := filepath.Join(t.TempDir(), "runner")
	if err := os.WriteFile(standIn, []byte("a stand-in for the runner"), 0o755); err != nil {
		t.Fatal(err)
	}
	releases := newRunnerReleases(t, standIn)
	release := releases.publish("initial")
	served := backendtest.New(t, backendtest.WithRunnerReleases(releases.dir)).Start()
	installer := served.Get("/install.sh", nil).Expect(http.StatusOK)
	headerIs(t, installer, "Content-Type", "text/x-shellscript; charset=utf-8")
	headerIs(t, installer, "Cache-Control", "no-store")
	contains(t, installer.Text(), "backend="+served.URL+"/")
	for _, wrong := range []string{
		release + "/" + target + "/demi-runner.exe",
		release + "/aarch64-apple-ios/demi-runner",
		sha([]byte("unpublished")) + "/" + target + "/demi-runner",
		release[:16] + "/" + target + "/demi-runner",
	} {
		served.Get("/runner-artifacts/"+wrong, nil).Expect(http.StatusNotFound)
	}
	executable := served.Get("/runner-artifacts/"+release+"/"+target+"/demi-runner", nil).Expect(http.StatusOK)
	if len(executable.Body) != releases.artifact["size"].(int) {
		t.Fatalf("the executable is %d bytes", len(executable.Body))
	}
	b.Stop()
	served.Stop()
}

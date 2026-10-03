package backend_test

import (
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// publishHostRelease makes the runner release fixture through its generated contract.
func publishHostRelease(t *testing.T, directory, program, name string) string {
	t.Helper()
	data, err := os.ReadFile(program)
	if err != nil {
		t.Fatal(err)
	}
	release := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	executable := filepath.Join(directory, release, string(target), "demi-runner")
	if err := os.MkdirAll(filepath.Dir(executable), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(program, executable); err != nil {
		t.Fatal(err)
	}
	targets := make(map[string]commandwire.PackageArtifact)
	for _, target := range commandwire.Targets {
		targets[target] = commandwire.PackageArtifact{SHA256: fmt.Sprintf("%x", sha256.Sum256(data)), Size: uint64(len(data))}
	}
	manifest, err := contract.EncodeJSON(runnerwire.RunnerRelease{Release: release, Wire: runnerwire.Version, CommandProtocol: commandwire.Version, Targets: targets})
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{filepath.Join(directory, release, "manifest.json"), filepath.Join(directory, "manifest.json")} {
		if err := os.WriteFile(path, manifest, 0644); err != nil {
			t.Fatal(err)
		}
	}
	return release
}

func TestInstallerWithoutReleasesAndArtifactAllowlist(t *testing.T) {
	h, _, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	b, err := h.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	script, err := b.Read(t.Context(), "/install.sh", nil)
	if err != nil {
		t.Fatal(err)
	}
	if script.Status != 503 || string(script.Body) != "Runner releases are not configured on this backend.\n" {
		t.Fatalf("installer: %d %s", script.Status, script.Body)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	initial := fmt.Sprintf("%x", sha256.Sum256([]byte("initial")))
	artifact, err := b.Read(t.Context(), fmt.Sprintf("/runner-artifacts/%s/%s/demi-runner", initial, target), nil)
	if err != nil {
		t.Fatal(err)
	}
	if artifact.Status != 404 {
		t.Fatal(artifact.Status)
	}
	program := filepath.Join(t.TempDir(), "stand-in")
	const bytes = "a stand-in for the runner"
	if err := os.WriteFile(program, []byte(bytes), 0644); err != nil {
		t.Fatal(err)
	}
	directory := t.TempDir()
	release := publishHostRelease(t, directory, program, "initial")
	servedHarness, _, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	servedHarness.Config.RunnerReleases = directory
	served, err := servedHarness.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	script, err = served.Read(t.Context(), "/install.sh", nil)
	if err != nil {
		t.Fatal(err)
	}
	if script.Headers.Get("Content-Type") != "text/x-shellscript; charset=utf-8" || script.Headers.Get("Cache-Control") != "no-store" {
		t.Fatal(script.Headers)
	}
	if !strings.Contains(string(script.Body), "backend="+served.URL+"/") {
		t.Fatal(string(script.Body))
	}
	for _, wrong := range []string{
		fmt.Sprintf("%s/%s/demi-runner.exe", release, target),
		release + "/aarch64-apple-ios/demi-runner",
		fmt.Sprintf("%x/%s/demi-runner", sha256.Sum256([]byte("unpublished")), target),
		fmt.Sprintf("%s/%s/demi-runner", release[:16], target),
	} {
		answer, err := served.Read(t.Context(), "/runner-artifacts/"+wrong, nil)
		if err != nil {
			t.Fatal(err)
		}
		if answer.Status != 404 {
			t.Fatalf("%s: %d", wrong, answer.Status)
		}
	}
	executable, err := served.Read(t.Context(), fmt.Sprintf("/runner-artifacts/%s/%s/demi-runner", release, target), nil)
	if err != nil {
		t.Fatal(err)
	}
	if executable.Status != 200 || len(executable.Body) != len(bytes) {
		t.Fatalf("executable: %d %d", executable.Status, len(executable.Body))
	}
}

// hostInstallations owns every runner launched by an installer, draining each on cleanup.
type hostInstallations struct {
	t      *testing.T
	home   string
	states []string
}

func newHostInstallations(t *testing.T) *hostInstallations {
	i := &hostInstallations{t: t, home: t.TempDir()}
	t.Cleanup(func() {
		for _, state := range i.states {
			launcher := filepath.Join(state, "run")
			if _, err := os.Stat(launcher); os.IsNotExist(err) {
				continue
			}
			cmd := exec.Command(launcher, "drain")
			if output, err := cmd.CombinedOutput(); err != nil {
				t.Errorf("drain %s: %v: %s", state, err, output)
			}
		}
	})
	return i
}
func (i *hostInstallations) state(b *backendtest.TestBackend) string {
	state := filepath.Join(i.home, ".demi/instances", fmt.Sprintf("%x", sha256.Sum256([]byte(b.URL+"/"))))
	i.states = append(i.states, state)
	return state
}
func (i *hostInstallations) install(b *backendtest.TestBackend, mask, installation string) ([]byte, error) {
	i.t.Helper()
	script, err := b.Read(i.t.Context(), "/install.sh", nil)
	if err != nil {
		i.t.Fatal(err)
	}
	if script.Status != 200 {
		i.t.Fatalf("installer: %d %s", script.Status, script.Body)
	}
	path := filepath.Join(i.home, fmt.Sprintf("install-%d.sh", b.Address().Port()))
	if err := os.WriteFile(path, script.Body, 0600); err != nil {
		i.t.Fatal(err)
	}
	command := exec.CommandContext(i.t.Context(), "sh", path)
	if mask != "" {
		command = exec.CommandContext(i.t.Context(), "sh", "-c", `umask "$1" && exec sh "$0"`, path, mask)
	}
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "HOME=") && !strings.HasPrefix(entry, "DEMI_INSTALLATION_ID=") {
			command.Env = append(command.Env, entry)
		}
	}
	command.Env = append(command.Env, "HOME="+i.home)
	if installation != "" {
		command.Env = append(command.Env, "DEMI_INSTALLATION_ID="+installation)
	}
	return command.CombinedOutput()
}

// activeHostField reads the same dynamic JSON value Rust observes, preserving
// generic JSON validation and using the contract runtime for each string value.
func activeHostField(t *testing.T, state, name string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(state, "active.json"))
	if err != nil {
		t.Fatal(err)
	}
	fields, err := contract.ObjectFields(data)
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range fields {
		if field.Name != name {
			continue
		}
		raw, ok := field.Value.(json.RawMessage)
		if !ok {
			t.Fatal("active field is not JSON")
		}
		value, err := contract.Decode[string](raw)
		if err != nil {
			t.Fatal(err)
		}
		return value
	}
	t.Fatalf("active runner has no %s", name)
	return ""
}

// Three real installs download and verify the runner; an upgrade drains only its own installation.
func TestInstallerSeparatesBackendsReusesReleaseAndUpgradesOwnRunner(t *testing.T) {
	program, err := programtest.Path(t.Context(), "demi-runner")
	if err != nil {
		t.Fatal(err)
	}
	directory := t.TempDir()
	initial := publishHostRelease(t, directory, program, "initial")
	start := func() *backendtest.TestBackend {
		h, _, err := backendtest.HostsHarness(t.Context(), t)
		if err != nil {
			t.Fatal(err)
		}
		h.Config.RunnerReleases = directory
		b, err := h.Start(t.Context(), t)
		if err != nil {
			t.Fatal(err)
		}
		return b
	}
	a, b := start(), start()
	installs := newHostInstallations(t)
	stateA, stateB := installs.state(a), installs.state(b)
	windows, err := a.Read(t.Context(), "/install.ps1", nil)
	if err != nil {
		t.Fatal(err)
	}
	if windows.Status != 200 || windows.Headers.Get("Content-Type") != "text/plain; charset=utf-8" || !strings.Contains(string(windows.Body), "aarch64-pc-windows-msvc") {
		t.Fatalf("Windows installer: %d %s", windows.Status, windows.Body)
	}
	for _, backend := range []*backendtest.TestBackend{a, b} {
		if output, err := installs.install(backend, "", ""); err != nil {
			t.Fatalf("install: %v: %s", err, output)
		}
	}
	firstA, firstB := activeHostField(t, stateA, "endpoint"), activeHostField(t, stateB, "endpoint")
	if firstA == firstB || activeHostField(t, stateA, "release") != initial {
		t.Fatal("installations not independent")
	}
	again, err := installs.install(a, "", "")
	if err != nil || !strings.Contains(string(again), "already running") {
		t.Fatalf("repeat: %v: %s", err, again)
	}
	if activeHostField(t, stateA, "endpoint") != firstA {
		t.Fatal("repeat replaced runner")
	}
	registration := fmt.Sprintf("%x", sha256.Sum256([]byte(a.URL+"/")))
	collision, err := installs.install(b, "", registration)
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 1 || !strings.Contains(string(collision), "another backend") {
		t.Fatalf("collision: %v: %s", err, collision)
	}
	upgraded := publishHostRelease(t, directory, program, "upgraded")
	if output, err := installs.install(a, "", ""); err != nil {
		t.Fatalf("upgrade: %v: %s", err, output)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	old, err := a.Send(t.Context(), "HEAD", fmt.Sprintf("/runner-artifacts/%s/%s/demi-runner", initial, target), nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if old.Status != 200 || old.Headers.Get("Cache-Control") != "public, max-age=31536000, immutable" {
		t.Fatal(old)
	}
	if activeHostField(t, stateA, "release") != upgraded || activeHostField(t, stateA, "endpoint") == firstA || activeHostField(t, stateB, "endpoint") != firstB {
		t.Fatal("upgrade changed wrong installation")
	}
}

// The real installer and a scripted model prove a user's shell mask reaches the installed runner's jobs.
func TestInstalledRunnerPreservesInvokingShellMask(t *testing.T) {
	program, err := programtest.Path(t.Context(), "demi-runner")
	if err != nil {
		t.Fatal(err)
	}
	releases := t.TempDir()
	publishHostRelease(t, releases, program, "initial")
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.RunnerReleases = releases
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	installs := newHostInstallations(t)
	state := installs.state(b)
	if output, err := installs.install(b, "002", ""); err != nil {
		t.Fatalf("install: %v: %s", err, output)
	}
	for path, want := range map[string]os.FileMode{state: 0700, filepath.Join(state, "runner.log"): 0600} {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != want {
			t.Fatalf("%s: %v, want %v", path, info.Mode().Perm(), want)
		}
	}
	code, err := backendtest.HostsInstalledCode(s.ctx, state)
	if err != nil {
		t.Fatal(err)
	}
	claimed := s.request("POST", "/api/devices/claim", fmt.Sprintf(`{"code":%q}`, code), 201)
	device, err := webapi.DecodeClaimedDevice(claimed.Body)
	if err != nil {
		t.Fatal(err)
	}
	if err := b.UntilOnline(s.ctx, &user, device.Device.ID, true); err != nil {
		t.Fatal(err)
	}
	w := s.work(cloudFirst)
	s.request("PATCH", "/api/conversations/"+cloudFirst, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, device.Device.ID, installs.home), 200)
	result := w.turn("mask", "umask; echo made > made.txt", "done", 60000)
	if !strings.Contains(result, "0002") {
		t.Fatal(result)
	}
	info, err := os.Stat(filepath.Join(installs.home, "made.txt"))
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0664 {
		t.Fatal(info.Mode())
	}
}

//go:build linux

package sandbox

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
)

func TestVersionMatchesExactly(t *testing.T) {
	release, err := DecodeRuntimeRelease(releaseManifest)
	if err != nil || release.Upstream == "" || release.Arm64Version == "" {
		t.Fatalf("pinned manifest: %+v, %v", release, err)
	}
	version := PinnedRelease().Version()
	for _, test := range []struct {
		output string
		want   bool
	}{
		{"runsc version " + version + "\nspec: 1.1.0\n", true},
		{"runsc version " + version + "\r\n", true},
		{"runsc version " + version + "\r", false},
		{"runsc version " + version + ".1\n", false},
		{"spec: 1.1.0\nrunsc version " + version + "\n", false},
	} {
		if got := ReportsVersion(test.output, version); got != test.want {
			t.Errorf("version %q: %v", test.output, got)
		}
	}
	if ReportsVersion("runsc version release-20260914.0-demi.1\n", "release-20260914.0") {
		t.Fatal("accepted patched version as upstream")
	}
}

func TestListingContainerStatus(t *testing.T) {
	for _, empty := range []string{"null", "[]"} {
		_, found, err := StatusIn([]byte(empty), "demi-a")
		if err != nil || found {
			t.Fatalf("empty status = %v, %v", found, err)
		}
	}
	status, found, err := StatusIn(
		[]byte(`[{"id":"demi-b","pid":7,"status":"running"},{"id":"demi-a","pid":9,"status":"paused","bundle":"/x"}]`),
		"demi-a",
	)
	if err != nil || !found || status != Paused {
		t.Fatalf("listing status = %v, %v, %v", status, found, err)
	}
	for _, invalid := range []string{`[{"id":"demi-a","status":"exploded"}]`, `[{"id":"demi-a"}]`} {
		if _, _, err := StatusIn([]byte(invalid), "demi-a"); err == nil {
			t.Errorf("accepted %s", invalid)
		}
	}
}

func TestCommandIncludesRootAndProfile(t *testing.T) {
	runsc := NewRunsc(systemtest.Placeholder(), "/run/demi-machine-manager", true)
	expected := []string{
		"--root=/run/demi-machine-manager/runsc",
		"--platform=systrap",
		"--network=sandbox",
		"--overlay2=none",
		"--file-access=shared",
		"--file-access-mounts=shared",
		"--allow-suid=true",
		"--directfs=true",
		"pause",
		"demi-a",
	}
	if got := runsc.Args([]string{"pause", "demi-a"}); !reflect.DeepEqual(got, expected) {
		t.Fatalf("args = %q", got)
	}
	runsc = NewRunsc(systemtest.Placeholder(), "/run/demi-machine-manager", false)
	expected = append(expected[:8], "--ignore-cgroups", "pause", "demi-a")
	if got := runsc.Args([]string{"pause", "demi-a"}); !reflect.DeepEqual(got, expected) {
		t.Fatalf("limits-off args = %q", got)
	}
}

// fixtureRuntime supplies this already-built test executable as a scripted
// runsc. The fixture dispatches before testing flags are parsed in TestMain.
func fixtureRuntime(t *testing.T, runtime string) *Runsc {
	t.Helper()
	// Instrumented fixture children exit before testing.M.Run and therefore need
	// an explicit coverage destination; otherwise warnings pollute their protocol.
	if testing.CoverMode() != "" && os.Getenv("GOCOVERDIR") == "" {
		t.Setenv("GOCOVERDIR", t.TempDir())
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	runsc := NewRunsc(system.NewTools(map[system.Tool]string{system.Runsc: executable}), runtime, false)
	if err := os.MkdirAll(runsc.Root(), 0o700); err != nil {
		t.Fatal(err)
	}
	return runsc
}

// waitConnection observes the wait subprocess's protocol event, never a delay.
func waitConnection(t *testing.T, listener *net.UnixListener) net.Conn {
	t.Helper()
	conn, err := listener.Accept()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := conn.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	})
	var ready [1]byte
	if _, err := io.ReadFull(conn, ready[:]); err != nil || ready[0] != 'W' {
		t.Fatalf("wait readiness = %q, %v", ready, err)
	}
	return conn
}

func TestWaitCancellationKeepsOwnedWaiter(t *testing.T) {
	runsc := fixtureRuntime(t, t.TempDir())
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: filepath.Join(runsc.Root(), "control"), Net: "unix"})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := listener.Close(); err != nil {
			t.Error(err)
		}
	}()
	sandbox := Recorded(Config{}, Dependencies{Runsc: runsc}, Record{ID: "demi-a"}, "demi-1")
	sandbox.startWaiting(t.Context())
	defer func() {
		if err := sandbox.StopWaiting(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	conn := waitConnection(t, listener)
	canceled, cancel := context.WithCancel(t.Context())
	cancel()
	if err := sandbox.Exited(canceled); !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled exit = %v", err)
	}
	if _, err := conn.Write([]byte("exit")); err != nil {
		t.Fatal(err)
	}
	if err := sandbox.Exited(t.Context()); err != nil {
		t.Fatal(err)
	}
	sandbox.startWaiting(t.Context())
	conn = waitConnection(t, listener)
	if err := sandbox.StopWaiting(canceled); err != nil {
		t.Fatal(err)
	}
	var data [1]byte
	if _, err := conn.Read(data[:]); !errors.Is(err, io.EOF) {
		t.Fatalf("wait child not reaped: %v", err)
	}
}

func TestRuntimeStartFailureReportsLog(t *testing.T) {
	runsc := fixtureRuntime(t, t.TempDir())
	if err := os.WriteFile(filepath.Join(runsc.Root(), "fail-start"), []byte("failure"), 0o600); err != nil {
		t.Fatal(err)
	}
	log, err := os.CreateTemp(t.TempDir(), "runtime-log")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := log.Close(); err != nil {
			t.Error(err)
		}
	}()
	err = runsc.Start(t.Context(), "demi-a", "/bundle", log, log.Name())
	if err == nil || err.Error() != "Cloud start failed: fixture failed start\n" {
		t.Fatalf("start error = %v", err)
	}
}

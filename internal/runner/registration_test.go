package runner

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerwire"
)

// About a second: real runner, a declared shell builtin and two management clients.
func TestDeclaredBuiltinAndDrainReleaseInstallation(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	if f.hello.Protocol != runnerwire.Version {
		t.Fatal("wrong runner protocol")
	}
	page := f.readLog(nil, 10, new("runner"))
	if page.Next == 0 {
		t.Fatal("startup log has no cursor")
	}
	started := false
	for _, line := range page.Lines {
		if line.Source != "runner" {
			t.Fatalf("unexpected log source: %q", line.Source)
		}
		started = started || line.Text == "runner test-release started"
	}
	if !started {
		t.Fatal("missing startup log")
	}
	fixture, err := os.ReadFile("testdata/declared-help.json")
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := runnerwire.DecodeManifest(fixture)
	if err != nil {
		t.Fatal(err)
	}
	data, err := manifest.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	f.send(&runnerwire.ManifestMessage{Manifest: data})
	f.send(&runnerwire.JobStart{JobID: "job", ManifestHash: &manifest.Hash, Context: runnerCommandContext(), Script: "fixture --help && printf done", CWD: f.home, Env: map[string]string{}})
	out, stderr, exit := f.jobOutput("job")
	requireJobSuccess(t, exit, stderr)
	if !strings.Contains(out, "fixture: Remote declaration") || !strings.HasSuffix(out, "done") {
		t.Fatalf("builtin output: %s", out)
	}
	if exit.Output == nil || exit.Output.StdoutBytes != uint64(len(out)) {
		t.Fatal("wrong output length")
	}
	if output, err := f.management(f.ctx, "status"); err != nil {
		t.Fatalf("status: %v: %s", err, output)
	}
	if output, err := f.management(f.ctx, "status", "--release", "different"); err == nil {
		t.Fatalf("release mismatch accepted: %s", output)
	} else {
		var exit *exec.ExitError
		if !errors.As(err, &exit) || exit.ExitCode() != 3 {
			t.Fatalf("status mismatch: %v", err)
		}
	}
	drained := make(chan error, 1)
	go func() {
		_, err := f.management(f.ctx, "drain")
		drained <- err
	}()
	_, _, closeErr := f.socket.Read(f.ctx)
	if err := <-drained; err != nil {
		t.Fatal(err)
	}
	if websocket.CloseStatus(closeErr) != websocket.StatusGoingAway {
		t.Fatalf("drain close: %v", closeErr)
	}
	if err := <-f.done; err != nil {
		t.Fatal(err)
	}
	f.command = nil
	if _, err := os.Stat(filepath.Join(f.state, "active.json")); !os.IsNotExist(err) {
		t.Fatal("active record survived drain")
	}
	if _, err := f.management(f.ctx, "status"); err == nil {
		t.Fatal("inactive installation reported active")
	} else {
		var exit *exec.ExitError
		if !errors.As(err, &exit) || exit.ExitCode() != 1 {
			t.Fatalf("inactive status: %v", err)
		}
	}
}

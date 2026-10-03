//go:build darwin || linux

package remotehosttest

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Bind the production socket suffix under the fixture's actual command TMPDIR.
func TestRunnerTemporaryDirectoryFitsLocalSocket(t *testing.T) {
	temporary, err := runnerTempDir(t)
	if err != nil {
		t.Fatal(err)
	}
	p := &RunnerProcess{binary: "unused", state: t.TempDir(), home: t.TempDir(), temporary: temporary}
	if err := os.Mkdir(filepath.Join(p.state, "tmp"), 0o700); err != nil {
		t.Fatal(err)
	}
	command := p.Command(context.Background())
	var root string
	for _, entry := range command.Env {
		if value, ok := strings.CutPrefix(entry, "TMPDIR="); ok {
			root = value
		}
	}
	directory, err := os.MkdirTemp(root, "demi-")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := os.RemoveAll(directory); err != nil {
			t.Error(err)
		}
	}()
	listener, err := net.Listen("unix", filepath.Join(directory, "ipc.sock"))
	if err != nil {
		t.Fatal(err)
	}
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
}

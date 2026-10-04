//go:build linux

package storage_test

import (
	"context"
	"flag"
	"os"
	"os/exec"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanager/system/systemtest"
	"go.uber.org/goleak"
)

var rootTests = flag.Bool(
	"storage-root",
	false,
	"run storage tests in private Linux namespaces (requires root and filesystem tools)",
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// isolatedStorage runs privileged disk scenarios on an owned namespace thread.
// Scenario defers run on that same thread before the namespace disappears.
func isolatedStorage(t *testing.T, job func(context.Context) error) {
	t.Helper()
	if !*rootTests {
		t.Skip("opt in with -storage-root; requires root, loop devices and filesystem tools")
	}
	if err := systemtest.Isolate(t.Context(), job); err != nil {
		t.Fatal(err)
	}
}

// storageTools resolves only the system-owned tool catalog used by storage.
func storageTools(t *testing.T) *system.Tools {
	t.Helper()
	tools, err := systemtest.OnPath(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return tools
}

// requireStorage keeps filesystem scenario setup failures next to their call sites.
func requireStorage(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

// diskCommand runs an independent filesystem tool that the scenarios use as their oracle.
func diskCommand(ctx context.Context, program string, args ...string) ([]byte, error) {
	command := exec.CommandContext(ctx, program, args...)
	command.Env = append(os.Environ(), "LC_ALL=C.UTF-8")
	return command.Output()
}

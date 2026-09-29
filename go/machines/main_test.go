//go:build linux

package machines_test

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/machines/internal/roottest"
)

func TestMain(m *testing.M) {
	code := roottest.Main(m)
	removeBuiltManager()
	os.Exit(code)
}

// The executable is shared with the process suite. Its first build can exceed
// one second; the two configuration refusals themselves need no root or host.
func TestMainMapsConfigurationAndUsageErrorsToExitStatus(t *testing.T) {
	executable := manager(t)
	for _, tc := range []struct {
		name    string
		args    []string
		env     []string
		status  int
		message string
	}{
		{name: "configuration", env: []string{"DEMI_MANAGED_FIRECRACKER=/obsolete"}, status: 1, message: "DEMI_MANAGED_FIRECRACKER is not a Cloud manager setting"},
		{name: "usage", args: []string{"--nonsense"}, status: 2, message: "flag provided but not defined"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
			defer cancel()
			command := exec.CommandContext(ctx, executable, tc.args...)
			command.Env = append([]string{"PATH=/usr/bin:/bin"}, tc.env...)
			output, err := command.CombinedOutput()
			var exit *exec.ExitError
			if !errors.As(err, &exit) || exit.ExitCode() != tc.status {
				t.Fatalf("exit: %v, want status %d; output: %s", err, tc.status, output)
			}
			if !strings.Contains(string(output), tc.message) {
				t.Fatalf("wrong refusal: %s", output)
			}
		})
	}
}

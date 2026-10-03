package backend_test

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/programtest"
)

func TestShutdownClosesListener(t *testing.T) {
	h, _, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	b, err := h.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if _, err := b.Read(t.Context(), "/api/setup", nil); err == nil {
		t.Fatal("backend answers after shutdown")
	}
}

// startupRefusal runs the backend with exactly the configuration under test.
// Building the program is shared by programtest; each invocation exits at configuration validation.
func startupRefusal(t *testing.T, variable, value string, omit bool) string {
	t.Helper()
	path, err := programtest.Path(t.Context(), "demi-backend")
	if err != nil {
		t.Fatal(err)
	}
	env := []string{"DEMI_INSTANCE_MODE=shared", "DEMI_BACKEND_PUBLIC_URL=http://127.0.0.1:3271", "DEMI_MACHINE_MANAGER_SOCKET=/nonexistent/demi-machine-manager.sock", "DEMI_NATIVE_CONFIG=/nonexistent/native.json"}
	if omit {
		for i, entry := range env {
			if strings.HasPrefix(entry, variable+"=") {
				env = append(env[:i], env[i+1:]...)
				break
			}
		}
	} else {
		env = append(env, variable+"="+value)
	}
	env = append(env, "DEMI_BACKEND_DATA="+t.TempDir())
	cmd := exec.CommandContext(t.Context(), path)
	cmd.Env = env
	output, err := cmd.CombinedOutput()
	var exit *exec.ExitError
	if !errors.As(err, &exit) {
		t.Fatalf("startup: %v: %s", err, output)
	}
	if !strings.Contains(string(output), variable) {
		t.Fatalf("does not name %s: %s", variable, output)
	}
	return string(output)
}

func TestInvalidPortNamesVariable(t *testing.T) { startupRefusal(t, "DEMI_BACKEND_PORT", "abc", false) }
func TestUnknownVariableNamesVariable(t *testing.T) {
	startupRefusal(t, "DEMI_BACKEND_PORTT", "3272", false)
}
func TestMissingPublicURLNamesVariable(t *testing.T) {
	startupRefusal(t, "DEMI_BACKEND_PUBLIC_URL", "", true)
}
func TestMalformedSecretIsNotDisclosed(t *testing.T) {
	output := startupRefusal(t, "DEMI_INSTANCE_SECRET", "not-a-hex-secret-value", false)
	if strings.Contains(output, "not-a-hex-secret-value") {
		t.Fatal("secret disclosed")
	}
}
func TestInstanceSecretPersistsWithPrivatePermissions(t *testing.T) {
	h, _, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	b, err := h.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(h.DataDir(), "instance-secret")
	created, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0600 {
		t.Fatalf("permissions: %v", info.Mode())
	}
	b, err = h.Start(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	kept, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(kept) != string(created) {
		t.Fatal("secret changed")
	}
	if err := os.WriteFile(path, []byte("not hex\n"), 0600); err != nil {
		t.Fatal(err)
	}
	started, err := backend.Start(t.Context(), h.Config)
	if err == nil {
		_ = started.Close(context.Background())
		t.Fatal("corrupt secret accepted")
	}
	var secret *backend.SecretError
	if !errors.As(err, &secret) || secret.Kind != backend.SecretCorrupt {
		t.Fatalf("corrupt secret: %v", err)
	}
}
func TestConfiguredSecretRequires64HexDigits(t *testing.T) {
	digits := strings.Repeat("0123456789abcdef", 4)
	for _, text := range []string{digits, strings.ToUpper(digits)} {
		if _, err := backend.ParseInstanceSecret(text); err != nil {
			t.Fatal(err)
		}
	}
	for _, text := range []string{digits[1:], digits[2:] + "zz"} {
		if _, err := backend.ParseInstanceSecret(text); err == nil {
			t.Fatal("invalid secret accepted")
		}
	}
}

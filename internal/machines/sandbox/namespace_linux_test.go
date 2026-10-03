//go:build linux

package sandbox

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
)

// TestSavedNamespaceRecoveryInheritsMountsAndLock starts a second copy of this
// built test binary in the saved namespace. It normally costs under one second.
func TestSavedNamespaceRecoveryInheritsMountsAndLock(t *testing.T) {
	if testing.CoverMode() != "" && os.Getenv("GOCOVERDIR") == "" {
		t.Setenv("GOCOVERDIR", t.TempDir())
	}
	if os.Geteuid() != 0 {
		t.Skip("needs an explicit root invocation of the Linux suite")
	}
	root := t.TempDir()
	runtime := filepath.Join(root, "runtime")
	project := filepath.Join(root, "project")
	for _, path := range []string{runtime, project} {
		if err := os.Mkdir(path, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	lockPath := filepath.Join(root, "manager.lock")
	lock, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := lock.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		t.Fatal(err)
	}
	namespace := NewSavedNamespace(runtime, root)
	defer func() {
		ctx := context.Background()
		if err := namespace.Release(ctx); err != nil {
			t.Error(err)
		}
		// Pin makes this disposable runtime root a private host-side bind mount.
		_, err := system.RunNamespace(ctx, system.HostMount(), func(ctx context.Context) (struct{}, error) {
			return struct{}{}, system.Unmount(ctx, runtime)
		})
		if err != nil {
			t.Error(err)
		}
	}()
	marker := filepath.Join(project, "marker")
	if err := systemtest.Isolate(t.Context(), func(ctx context.Context) error {
		if err := namespace.Pin(ctx); err != nil {
			return err
		}
		if err := system.Tmpfs(ctx, project, "size=1m"); err != nil {
			return err
		}
		// This namespace deliberately outlives its creating thread via the pin.
		return os.WriteFile(marker, []byte("inside saved namespace"), 0o600)
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(marker); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("saved mount leaked into caller: %v", err)
	}
	wrong := NewSavedNamespace(runtime, root+"-other")
	if err := wrong.Recover(
		t.Context(),
		lock,
		nil,
	); err == nil ||
		!strings.Contains(err.Error(), "Recover the previous Cloud manager with its original state directory: ") {
		t.Fatalf("other owner = %v", err)
	}
	args := []string{"--probe", marker, lockPath, "true", "--recover"}
	if err := namespace.Recover(
		t.Context(),
		lock,
		args,
	); err == nil ||
		!strings.Contains(err.Error(), "Cloud namespace recovery failed: ") {
		t.Fatalf("failed child = %v", err)
	}
	// A successful retry proves failure retained both the pin and its mount data.
	args[3] = "false"
	if err := namespace.Recover(t.Context(), lock, args); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"mount-namespace", "mount-namespace-owner.json"} {
		if _, err := os.Stat(filepath.Join(runtime, path)); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("namespace resource remains: %s: %v", path, err)
		}
	}
	if err := namespace.Recover(t.Context(), lock, nil); err != nil {
		t.Fatal("absent recovery", err)
	}
}

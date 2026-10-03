//go:build linux

package sandbox

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/machines/system"
)

// cgroupFixture masks the host hierarchy inside the test's mount namespace.
// Static files stand in for controller discovery and a permanently busy group;
// no test changes or kills anything in the VM's actual cgroup hierarchy.
func cgroupFixture(ctx context.Context, t *testing.T) {
	t.Helper()
	if err := system.Tmpfs(ctx, cgroupRoot, "size=1m"); err != nil {
		t.Fatal(err)
	}
}

func TestCgroupControllersCheckedBeforeChanges(t *testing.T) {
	isolatedSandbox(t, func(ctx context.Context) {
		cgroupFixture(ctx, t)
		defer func() {
			if err := system.Unmount(context.WithoutCancel(ctx), cgroupRoot); err != nil {
				t.Error(err)
			}
		}()
		if err := PrepareCgroups(
			ctx,
		); err == nil ||
			err.Error() != "Cloud resource limits need the cgroup v2 cpu, memory and pids controllers "+
				"at /sys/fs/cgroup; missing: cpu, memory, pids. DEMI_MANAGED_LIMITS=off runs Clouds without limits" {
			t.Fatalf("absent hierarchy = %v", err)
		}
		if err := os.WriteFile(filepath.Join(cgroupRoot, "cgroup.controllers"), []byte("cpu pids"), 0o600); err != nil {
			t.Fatal(err)
		}
		if err := PrepareCgroups(
			ctx,
		); err == nil ||
			err.Error() != "Cloud resource limits need the cgroup v2 cpu, memory and pids controllers "+
				"at /sys/fs/cgroup; missing: memory. DEMI_MANAGED_LIMITS=off runs Clouds without limits" {
			t.Fatalf("missing controllers = %v", err)
		}
		if _, err := os.Stat(filepath.Join(cgroupRoot, "cgroup.subtree_control")); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("changed hierarchy before validation: %v", err)
		}
		if err := os.WriteFile(
			filepath.Join(cgroupRoot, "cgroup.controllers"),
			[]byte("cpu memory pids"),
			0o600,
		); err != nil {
			t.Fatal(err)
		}
		if err := PrepareCgroups(ctx); err != nil {
			t.Fatal(err)
		}
		for _, path := range []string{"cgroup.subtree_control", "demi-cloud/cgroup.subtree_control"} {
			data, err := os.ReadFile(filepath.Join(cgroupRoot, path))
			if err != nil || string(data) != "+cpu +memory +pids" {
				t.Fatalf("controllers %s = %q, %v", path, data, err)
			}
		}
		if err := Fence(ctx, "demi-missing"); err != nil {
			t.Fatal(err)
		}
	})
}

func TestCgroupFenceDeadline(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		isolatedSandbox(t, func(ctx context.Context) {
			cgroupFixture(ctx, t)
			defer func() {
				if err := system.Unmount(context.WithoutCancel(ctx), cgroupRoot); err != nil {
					t.Error(err)
				}
			}()
			group := filepath.Join(cgroupRoot, "demi-cloud/demi-busy")
			if err := os.MkdirAll(group, 0o700); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(group, "cgroup.events"), []byte("populated 1\n"), 0o600); err != nil {
				t.Fatal(err)
			}
			start := time.Now()
			if err := Fence(ctx, "demi-busy"); !errors.Is(err, ErrWriters) {
				t.Fatalf("fence = %v", err)
			}
			if elapsed := time.Since(start); elapsed != 5*time.Second {
				t.Fatalf("fence deadline = %v", elapsed)
			}
			data, err := os.ReadFile(filepath.Join(group, "cgroup.kill"))
			if err != nil || string(data) != "1" {
				t.Fatalf("kill = %q, %v", data, err)
			}
		})
	})
}

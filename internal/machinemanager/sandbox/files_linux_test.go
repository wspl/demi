//go:build linux

package sandbox

import (
	"context"
	"errors"
	"net/netip"
	"os"
	"path/filepath"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanager/system/systemtest"
	"github.com/wspl/demi/internal/runnerproto"
)

// isolatedSandbox keeps mount cleanup on the namespace's owned thread. Root
// scenarios cost under one second unless they launch the real runtime.
func isolatedSandbox(t *testing.T, job func(context.Context)) {
	t.Helper()
	if os.Geteuid() != 0 {
		t.Skip("needs an explicit root invocation of the Linux suite")
	}
	if err := systemtest.Isolate(t.Context(), func(ctx context.Context) error {
		job(ctx)
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestCredentialModesIgnoreUmask(t *testing.T) {
	job := func(ctx context.Context) {
		// Only the isolated root scenario changes the thread-local mask.
		if os.Geteuid() == 0 {
			previous := syscall.Umask(0o77)
			defer syscall.Umask(previous)
		}
		directory := NewRuntimeDirectory(t.TempDir(), "demi-test")
		if err := directory.Create(ctx); err != nil {
			t.Fatal(err)
		}
		boot, err := runnerproto.DecodeManagedBoot(
			[]byte(`{"backendUrl":"https://backend.example.com","deviceToken":"tok"}`),
		)
		if err != nil {
			t.Fatal(err)
		}
		if err := directory.WriteCredentials(
			ctx,
			boot,
			[]netip.Addr{netip.MustParseAddr("1.1.1.1")},
		); err != nil &&
			(os.Geteuid() == 0 || !errors.Is(err, os.ErrPermission)) {
			t.Fatal(err)
		}
		for path, mode := range map[string]os.FileMode{
			directory.Boot():     0o400,
			directory.Resolver(): 0o444,
			directory.Hosts():    0o444,
			directory.Root():     0o700,
		} {
			info, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			if info.Mode().Perm() != mode {
				t.Errorf("%s mode = %o, want %o", path, info.Mode().Perm(), mode)
			}
		}
		info, err := os.Stat(directory.Boot())
		if err != nil {
			t.Fatal(err)
		}
		stat, ok := info.Sys().(*syscall.Stat_t)
		if os.Geteuid() == 0 && (!ok || stat.Uid != system.UserID || stat.Gid != system.UserID) {
			t.Fatalf("credential ownership: %+v", info.Sys())
		}
		resolver, err := os.ReadFile(directory.Resolver())
		if err != nil || string(resolver) != "nameserver 1.1.1.1\n" {
			t.Fatalf("resolver = %q, %v", resolver, err)
		}
		data, err := os.ReadFile(directory.Boot())
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := runnerproto.DecodeManagedBoot(data)
		if err != nil || decoded != boot {
			t.Fatalf("boot round trip failed: %v", err)
		}
	}
	if os.Geteuid() == 0 {
		isolatedSandbox(t, job)
	} else {
		job(t.Context())
	}
}

func TestRemovalPreservesSurvivingMount(t *testing.T) {
	isolatedSandbox(t, func(ctx context.Context) {
		directory := NewRuntimeDirectory(t.TempDir(), "demi-test")
		if err := directory.Create(ctx); err != nil {
			t.Fatal(err)
		}
		if err := system.Tmpfs(ctx, directory.Home(), "size=1m"); err != nil {
			t.Fatal(err)
		}
		defer func() {
			if mounted, _, err := system.MountRoot(context.WithoutCancel(ctx), directory.Home()); err != nil {
				t.Error(err)
			} else if mounted {
				if err := system.Unmount(context.WithoutCancel(ctx), directory.Home()); err != nil {
					t.Error(err)
				}
			}
		}()
		project := filepath.Join(directory.Home(), "project")
		if err := os.WriteFile(project, []byte("work"), 0o600); err != nil {
			t.Fatal(err)
		}
		if err := directory.Remove(ctx); err == nil {
			t.Fatal("removed live mount")
		}
		if data, err := os.ReadFile(project); err != nil || string(data) != "work" {
			t.Fatalf("mounted project = %q, %v", data, err)
		}
		if err := system.Unmount(ctx, directory.Home()); err != nil {
			t.Fatal(err)
		}
		if err := directory.Remove(ctx); err != nil {
			t.Fatal(err)
		}
	})
}

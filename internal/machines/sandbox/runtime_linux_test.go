//go:build linux

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// guestProbe runs as the initial UID 1000 OCI process. It proves the credential
// bind and writable home in the real runtime, then exits on runtime termination.
func guestProbe() error {
	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGTERM)
	defer cancel()
	if os.Getuid() != int(system.UserID) || os.Getgid() != int(system.UserID) {
		return fmt.Errorf("wrong guest identity: %d:%d", os.Getuid(), os.Getgid())
	}
	data, err := os.ReadFile(BootRecord)
	if err != nil {
		return err
	}
	if _, err := runnerwire.DecodeManagedBoot(data); err != nil {
		return err
	}
	if err := os.WriteFile("/home/demi/ready.tmp", []byte("guest running"), 0o600); err != nil {
		return err
	}
	// Publish only the complete record; existence is the parent's readiness event.
	if err := os.Rename("/home/demi/ready.tmp", "/home/demi/ready"); err != nil {
		return err
	}
	<-ctx.Done()
	return nil
}

// TestRealRunscLifecycle needs the pinned runsc. The static guest probe is this
// built test binary, installed as the OCI init; it needs no downloaded image.
// Expected cost with gVisor is several seconds for Sentry/Gofer startup and stop.
func TestRealRunscLifecycle(t *testing.T) {
	path, err := exec.LookPath("runsc")
	if err != nil {
		t.Skip(
			"runsc absent: pinned arm64 gVisor build runs in Phase 3 x-machines; real runtime lifecycle awaits x-machines",
		)
	}
	isolatedSandbox(t, func(ctx context.Context) {
		network := &networkFixture{}
		sandbox, working, base, boot, lease := bootFixture(ctx, t, network)
		sandbox.dependencies.Runsc = NewRunsc(
			system.NewTools(map[system.Tool]string{system.Runsc: path}),
			sandbox.config.Runtime,
			false,
		)
		defer cleanupBoot(ctx, t, sandbox, working, network)
		version, err := sandbox.dependencies.Runsc.Version(ctx)
		if err != nil {
			t.Fatal(err)
		}
		if !ReportsVersion(version, PinnedRelease().Version()) {
			t.Fatalf("runtime is not pinned: %s", version)
		}
		for _, directory := range []string{"usr/bin", "etc", "proc", "dev", "dev/pts", "dev/shm", "run", "tmp", "home"} {
			if err := os.MkdirAll(filepath.Join(base, directory), 0o755); err != nil {
				t.Fatal(err)
			}
		}
		for _, file := range []string{"etc/hosts", "etc/resolv.conf", "run/demi-boot.json"} {
			if err := os.WriteFile(filepath.Join(base, file), nil, 0o644); err != nil {
				t.Fatal(err)
			}
		}
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		init := filepath.Join(base, "usr/bin/tini")
		if err := fixtureCopy(ctx, executable, init); err != nil {
			t.Fatal(err)
		}
		if err := os.Chmod(init, 0o755); err != nil {
			t.Fatal(err)
		}
		home := filepath.Join(t.TempDir(), "home")
		if err := os.Mkdir(home, 0o700); err != nil {
			t.Fatal(err)
		}
		device, err := system.Attach(ctx, working.Image(machinewire.VolumeHome))
		if err != nil {
			t.Fatal(err)
		}
		mountErr := system.Ext4(ctx, device.Path(), home)
		if err := errors.Join(mountErr, device.Close()); err != nil {
			t.Fatal(err)
		}
		prepareErr := func() (err error) {
			defer func() { err = errors.Join(err, system.Unmount(context.WithoutCancel(ctx), home)) }()
			userHome := filepath.Join(home, "demi")
			if err := os.Mkdir(userHome, 0o755); err != nil {
				return err
			}
			return os.Chown(userHome, int(system.UserID), int(system.UserID))
		}()
		if prepareErr != nil {
			t.Fatal(prepareErr)
		}
		if err := os.MkdirAll("/run/netns", 0o755); err != nil {
			t.Fatal(err)
		}
		netns := "/run/netns/" + sandbox.slot.Namespace
		if err := os.WriteFile(netns, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		if err := system.Bind(ctx, "/proc/thread-self/ns/net", netns); err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := system.Unmount(context.WithoutCancel(ctx), netns); err != nil {
				t.Error(err)
			}
		}()
		if err := sandbox.Start(ctx, working, base, boot); err != nil {
			t.Fatal(err)
		}
		// Poll the guest's explicit readiness event; the deadline only guards a hang.
		deadline := time.Now().Add(20 * time.Second)
		for {
			data, err := os.ReadFile(filepath.Join(sandbox.directory.Home(), "demi/ready"))
			if err == nil {
				if string(data) != "guest running" {
					t.Fatalf("guest event = %q", data)
				}
				break
			}
			if !errors.Is(err, os.ErrNotExist) {
				t.Fatal(err)
			}
			if time.Now().After(deadline) {
				t.Fatal("guest did not publish readiness")
			}
			runtime.Gosched()
		}
		if err := sandbox.Pause(ctx); err != nil {
			t.Fatal(err)
		}
		if status, found, err := sandbox.Status(ctx); err != nil || !found || status != Paused {
			t.Fatalf("paused status = %s, %v, %v", status, found, err)
		}
		if err := sandbox.Close(ctx, working); err != nil {
			t.Fatal(err)
		}
		if lease.releases != 1 {
			t.Fatalf("lease releases = %d", lease.releases)
		}
	})
}

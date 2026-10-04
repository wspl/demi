//go:build linux

package system_test

import (
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanager/system/systemtest"
	"go.uber.org/goleak"
	"golang.org/x/sys/unix"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// isolated runs an explicitly root-invoked test with its mount cleanup on the
// namespace thread, before the thread exits. Each scenario normally costs <1 s.
func isolated(t *testing.T, job func(context.Context)) {
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

// makeImage creates the disposable ext4 fixture used by the manager's mount tests.
func makeImage(ctx context.Context, t *testing.T, path string) {
	t.Helper()
	tools, err := systemtest.OnPath(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := tools.Run(ctx, system.Mke2fs, []string{"-q", "-t", "ext4", "-F", path, "32m"}, 0); err != nil {
		t.Fatal(err)
	}
}

// mountImage holds the loop descriptor until the fixture filesystem is mounted.
func mountImage(ctx context.Context, t *testing.T, image, target string) {
	t.Helper()
	device, err := system.Attach(ctx, image)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := device.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := system.Ext4(ctx, device.Path(), target); err != nil {
		t.Fatal(err)
	}
}

// unmount checks fixture cleanup on the namespace thread, even on test failure.
func unmount(ctx context.Context, t *testing.T, target string) {
	t.Helper()
	if err := system.Unmount(context.WithoutCancel(ctx), target); err != nil {
		t.Error(err)
	}
}

func TestOverlayPreservesRootMode(t *testing.T) {
	isolated(t, func(ctx context.Context) {
		unix.Umask(0o077)
		dir := t.TempDir()
		base, volume, root := filepath.Join(dir, "base"), filepath.Join(dir, "volume"), filepath.Join(dir, "root")
		for _, path := range []string{base, volume, root} {
			if err := os.Mkdir(path, 0o700); err != nil {
				t.Fatal(err)
			}
		}
		image := filepath.Join(dir, "system.ext4")
		makeImage(ctx, t, image)
		mountImage(ctx, t, image, volume)
		defer unmount(ctx, t, volume)
		for _, mode := range []os.FileMode{0o755, 0o500} {
			func() {
				if err := system.Overlay(ctx, base, volume, root); err != nil {
					t.Fatal(err)
				}
				defer unmount(ctx, t, root)
				info, err := os.Stat(root)
				if err != nil {
					t.Fatal(err)
				}
				if info.Mode().Perm() != mode {
					t.Errorf("root mode = %o, want %o", info.Mode().Perm(), mode)
				}
				if err := os.Chmod(root, 0o500); err != nil {
					t.Fatal(err)
				}
			}()
		}
	})
}

func TestLoopDetachesAfterUnmountAndFailedMount(t *testing.T) {
	isolated(t, func(ctx context.Context) {
		dir := t.TempDir()
		image, target := filepath.Join(dir, "home.ext4"), filepath.Join(dir, "home")
		if err := os.Mkdir(target, 0o700); err != nil {
			t.Fatal(err)
		}
		makeImage(ctx, t, image)
		func() {
			mountImage(ctx, t, image, target)
			defer unmount(ctx, t, target)
			link := filepath.Join(dir, "image-link")
			if err := os.Symlink(image, link); err != nil {
				t.Fatal(err)
			}
			if attached, err := systemtest.LoopAttached(ctx, link); err != nil || !attached {
				t.Fatalf("mounted image attached = %v, %v", attached, err)
			}
			mounted, exists, err := system.MountRoot(ctx, target)
			if err != nil || !mounted || !exists {
				t.Fatalf("mount root = %v, %v, %v", mounted, exists, err)
			}
		}()
		if err := systemtest.WaitLoopDetach(ctx, image); err != nil {
			t.Fatal(err)
		}
		mounted, exists, err := system.MountRoot(ctx, target)
		if err != nil || mounted || !exists {
			t.Fatalf("unmounted root = %v, %v, %v", mounted, exists, err)
		}
		garbage := filepath.Join(dir, "garbage.img")
		file, err := os.Create(garbage)
		if err != nil {
			t.Fatal(err)
		}
		truncateErr := file.Truncate(8 << 20)
		closeErr := file.Close()
		if err := errors.Join(truncateErr, closeErr); err != nil {
			t.Fatal(err)
		}
		func() {
			device, err := system.Attach(ctx, garbage)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := device.Close(); err != nil {
					t.Error(err)
				}
			}()
			if err := system.Ext4(ctx, device.Path(), target); err == nil {
				t.Fatal("mounted non-ext4 image")
			}
		}()
		if err := systemtest.WaitLoopDetach(ctx, garbage); err != nil {
			t.Fatal(err)
		}
		canceled, cancel := context.WithCancel(ctx)
		cancel()
		if err := systemtest.WaitLoopDetach(canceled, image); !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled detach wait = %v", err)
		}
		mounted, exists, err = system.MountRoot(ctx, filepath.Join(dir, "absent"))
		if err != nil || mounted || exists {
			t.Fatalf("absent root = %v, %v, %v", mounted, exists, err)
		}
	})
}

func TestBindReadOnlyAndDetach(t *testing.T) {
	isolated(t, func(ctx context.Context) {
		dir := t.TempDir()
		source, target := filepath.Join(dir, "source"), filepath.Join(dir, "target")
		for _, path := range []string{source, target} {
			if err := os.Mkdir(path, 0o700); err != nil {
				t.Fatal(err)
			}
		}
		if err := system.Tmpfs(ctx, source, "size=1m"); err != nil {
			t.Fatal(err)
		}
		defer unmount(ctx, t, source)
		if err := os.WriteFile(filepath.Join(source, "file"), []byte("kept"), 0o600); err != nil {
			t.Fatal(err)
		}
		if err := system.Bind(ctx, source, target); err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := system.Detach(context.WithoutCancel(ctx), target); err != nil {
				t.Error(err)
			}
		}()
		if err := system.MakePrivate(ctx, target); err != nil {
			t.Fatal(err)
		}
		if err := system.RemountReadOnly(ctx, target); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(target, "new"), nil, 0o600); !errors.Is(err, unix.EROFS) {
			t.Fatalf("write to read-only bind: %v", err)
		}
		if data, err := os.ReadFile(filepath.Join(target, "file")); err != nil || string(data) != "kept" {
			t.Fatalf("bind read = %q, %v", data, err)
		}
	})
}

func TestLoopRefreshCapacity(t *testing.T) {
	isolated(t, func(ctx context.Context) {
		image := filepath.Join(t.TempDir(), "grow.img")
		file, err := os.Create(image)
		if err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := file.Close(); err != nil {
				t.Error(err)
			}
		}()
		if err := file.Truncate(8 << 20); err != nil {
			t.Fatal(err)
		}
		device, err := system.Attach(ctx, image)
		if err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := device.Close(); err != nil {
				t.Error(err)
			}
		}()
		if err := file.Truncate(16 << 20); err != nil {
			t.Fatal(err)
		}
		if err := system.RefreshCapacity(ctx, device.Number()); err != nil {
			t.Fatal(err)
		}
		block, err := os.Open(device.Path())
		if err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := block.Close(); err != nil {
				t.Error(err)
			}
		}()
		size, err := block.Seek(0, io.SeekEnd)
		if err != nil || size != 16<<20 {
			t.Fatalf("device capacity = %d, %v", size, err)
		}
	})
}

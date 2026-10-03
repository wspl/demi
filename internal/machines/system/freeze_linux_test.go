//go:build linux

package system_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
	"golang.org/x/sys/unix"
)

func TestFrozenThawsEveryPath(t *testing.T) {
	isolated(t, func(ctx context.Context) {
		dir := t.TempDir()
		var mounts []string
		for _, name := range []string{"system", "home"} {
			image, target := filepath.Join(dir, name+".ext4"), filepath.Join(dir, name)
			makeImage(ctx, t, image)
			if err := os.Mkdir(target, 0o700); err != nil {
				t.Fatal(err)
			}
			mountImage(ctx, t, image, target)
			defer unmount(ctx, t, target)
			mounts = append(mounts, target)
		}
		if thawed, err := system.Thaw(ctx, mounts[0]); err != nil || thawed {
			t.Fatalf("initial thaw = %v, %v", thawed, err)
		}
		for _, path := range []string{"success", "panic", "failed-freeze"} {
			func() {
				panicked := false
				defer func() {
					if value := recover(); value != nil {
						panicked = true
						if value != "copy failed" {
							t.Errorf("unexpected panic: %v", value)
						}
					}
					if panicked != (path == "panic") {
						t.Errorf("panic path = %v, scenario %s", panicked, path)
					}
				}()
				var frozen system.Frozen
				defer func() {
					if failures := frozen.ThawAll(context.WithoutCancel(ctx)); len(failures) != 0 {
						t.Errorf("deferred thaw: %v", failures)
					}
				}()
				for _, mount := range mounts {
					if err := frozen.Freeze(ctx, mount); err != nil {
						t.Fatal(err)
					}
					// A second freeze must see a genuinely frozen filesystem, rather
					// than allowing an omitted freeze to pass a thaw-only scenario.
					if err := system.Freeze(ctx, mount); !errors.Is(err, unix.EBUSY) {
						t.Fatalf("second freeze = %v, want EBUSY", err)
					}
				}
				if path == "panic" {
					panic("copy failed")
				}
				if path == "failed-freeze" {
					absent := filepath.Join(dir, "absent")
					if err := frozen.Freeze(ctx, absent); !errors.Is(err, os.ErrNotExist) {
						t.Fatalf("failed freeze = %v", err)
					}
					failures := frozen.ThawAll(ctx)
					if len(failures) != 1 || !errors.Is(failures[0], os.ErrNotExist) {
						t.Fatalf("recorded failed freeze thaw = %v", failures)
					}
					return
				}
				if failures := frozen.ThawAll(ctx); len(failures) != 0 {
					t.Fatal(failures)
				}
			}()
			for _, mount := range mounts {
				if thawed, err := system.Thaw(ctx, mount); err != nil || thawed {
					t.Fatalf("after %s: thaw = %v, %v", path, thawed, err)
				}
				if err := os.WriteFile(filepath.Join(mount, "written"), []byte("after"), 0o600); err != nil {
					t.Fatal(err)
				}
			}
		}
	})
}

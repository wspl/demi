//go:build linux

package storage_test

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/system"
)

// Cost: local IO and ownership syscalls, no subprocesses.
func TestSkeletonLinksAndOwnership(t *testing.T) {
	directory := t.TempDir()
	skeleton := filepath.Join(directory, "skel")
	requireStorage(t, os.MkdirAll(filepath.Join(skeleton, ".config"), 0o755))
	requireStorage(t, os.WriteFile(filepath.Join(skeleton, ".profile"), []byte("PATH=$HOME/.local/bin:$PATH\n"), 0o644))
	requireStorage(t, os.Symlink(".profile", filepath.Join(skeleton, ".bash_profile")))
	requireStorage(t, os.Symlink("/etc/bash.bashrc", filepath.Join(skeleton, ".bashrc")))
	home := filepath.Join(directory, "demi")
	check := func(ctx context.Context) error {
		copied := storage.CopySkeleton(ctx, skeleton, home)
		if os.Geteuid() != 0 {
			if !errors.Is(copied, os.ErrPermission) {
				return fmt.Errorf("non-root ownership: %v", copied)
			}
			return nil
		}
		if copied != nil {
			return copied
		}
		for name, want := range map[string]string{".bash_profile": ".profile", ".bashrc": "/etc/bash.bashrc"} {
			got, err := os.Readlink(filepath.Join(home, name))
			if err != nil {
				return err
			}
			if got != want {
				return fmt.Errorf("%s links to %s, want %s", name, got, want)
			}
		}
		for _, name := range []string{".", ".config", ".profile", ".bashrc"} {
			info, err := os.Lstat(filepath.Join(home, name))
			if err != nil {
				return err
			}
			stat, ok := info.Sys().(*syscall.Stat_t)
			if !ok {
				return fmt.Errorf("no Linux stat for %s", name)
			}
			if stat.Uid != system.UserID || stat.Gid != system.UserID {
				return fmt.Errorf("%s owned by %d:%d", name, stat.Uid, stat.Gid)
			}
		}
		return nil
	}
	if os.Geteuid() == 0 {
		isolatedStorage(t, check)
	} else {
		requireStorage(t, check(t.Context()))
	}
}

// Cost: opt-in home creation and three debugfs queries; normally <1 s.
func TestHomeImageUserDirectoryAndReadableRoot(t *testing.T) {
	directory := t.TempDir()
	tools := storageTools(t)
	isolatedStorage(t, func(ctx context.Context) error {
		skeleton := filepath.Join(directory, "skel")
		if err := os.Mkdir(skeleton, 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(skeleton, ".profile"), []byte("export EDITOR=vi\n"), 0o644); err != nil {
			return err
		}
		root := filepath.Join(directory, "mkhome")
		if err := os.Mkdir(root, 0o755); err != nil {
			return err
		}
		if err := os.Chmod(root, 0o755); err != nil {
			return err
		}
		if err := storage.CopySkeleton(ctx, skeleton, filepath.Join(root, "demi")); err != nil {
			return err
		}
		image := filepath.Join(directory, "home.ext4")
		if err := storage.MakeHome(ctx, tools, root, image, 32<<20); err != nil {
			return err
		}
		for _, test := range []struct {
			path string
			want []string
		}{
			{"/", []string{"Mode:  0755"}},
			{"/demi", []string{"User:  1000", "Group:  1000"}},
			{"/demi/.profile", []string{"User:  1000"}},
		} {
			output, err := diskCommand(ctx, "debugfs", "-R", "stat "+test.path, image)
			if err != nil {
				return err
			}
			for _, want := range test.want {
				if !strings.Contains(string(output), want) {
					return fmt.Errorf("%s lacks %s: %s", test.path, want, output)
				}
			}
		}
		return nil
	})
}

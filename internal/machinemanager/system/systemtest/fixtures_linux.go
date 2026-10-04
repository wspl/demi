//go:build linux

package systemtest

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/wspl/demi/internal/machinemanager/system"
	"golang.org/x/sys/unix"
)

// Isolate runs job on an owned, locked OS thread in new mount and network
// namespaces, with recursive private mounts and a fresh tmpfs over /run.
// It requires root and joins the job; tests must opt in before calling it.
func Isolate(ctx context.Context, job func(context.Context) error) error {
	if os.Geteuid() != 0 {
		return fmt.Errorf("root tests run as root: %w", unix.EPERM)
	}
	_, err := system.RunNamespace(ctx, system.NewNetwork(), func(ctx context.Context) (struct{}, error) {
		if err := unix.Unshare(unix.CLONE_NEWNS); err != nil {
			return struct{}{}, err
		}
		if err := unix.Mount("", "/", "", unix.MS_PRIVATE|unix.MS_REC, ""); err != nil {
			return struct{}{}, err
		}
		if err := system.Tmpfs(ctx, "/run", ""); err != nil {
			return struct{}{}, err
		}
		return struct{}{}, job(ctx)
	})
	return err
}

// Placeholder returns tools with empty paths for command-line-only tests.
func Placeholder() *system.Tools {
	return &system.Tools{}
}

// OnPath resolves infrastructure tools on PATH without requiring runsc.
func OnPath(ctx context.Context) (*system.Tools, error) {
	// Use this test executable only as the configured-path placeholder so the
	// production resolver remains the sole owner of PATH lookup behavior.
	executable, err := os.Executable()
	if err != nil {
		return nil, err
	}
	resolved, err := system.Resolve(ctx, executable)
	if err != nil {
		return nil, err
	}
	paths := map[system.Tool]string{system.Runsc: "runsc"}
	for _, tool := range []system.Tool{system.Mke2fs, system.E2fsck, system.Resize2fs, system.Bsdtar} {
		paths[tool] = resolved.Path(tool)
	}
	return system.NewTools(paths), nil
}

// NftPath resolves nft for tests that read back installed firewall tables.
// Production firewall changes use the nftables API, never this program.
func NftPath(ctx context.Context) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	path, err := exec.LookPath("nft")
	if errors.Is(err, exec.ErrDot) {
		path, err = filepath.Abs(path)
	}
	if err != nil {
		return "", fmt.Errorf("nft is not installed: %w", err)
	}
	return path, nil
}

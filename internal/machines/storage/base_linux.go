//go:build linux

package storage

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
	"golang.org/x/sys/unix"
)

// ImportBase verifies and imports the release in image into bases unless it
// is already there, and returns its manifest digest as the base version.
// It never executes image content on the host.
func ImportBase(ctx context.Context, tools *system.Tools, image, bases string) (machinewire.BaseVersion, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	data, err := os.ReadFile(filepath.Join(image, "manifest.json"))
	if err != nil {
		return "", err
	}
	version, err := machinewire.ParseBaseVersion(fmt.Sprintf("%x", sha256.Sum256(data)))
	if err != nil {
		return "", err
	}
	manifest, err := machinewire.DecodeCloudImageManifest(data)
	if err != nil {
		return "", err
	}
	architecture, supported := machinewire.HostArchitecture()
	if !supported || architecture != manifest.Architecture {
		return "", ErrArchitecture
	}
	for _, required := range []string{machinewire.RunnerPath, machinewire.InitPath} {
		if _, ok := manifest.Executables[required]; !ok {
			return "", &MissingExecutableError{Path: required}
		}
	}
	if err := CreatePrivate(ctx, bases); err != nil {
		return "", err
	}
	entries, err := os.ReadDir(bases)
	if err != nil {
		return "", err
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), ".base-") {
			if err := RemoveTree(ctx, filepath.Join(bases, entry.Name())); err != nil {
				return "", err
			}
		}
	}
	target := filepath.Join(bases, string(version))
	saved, err := os.ReadFile(filepath.Join(target, "manifest.json"))
	if err == nil {
		if !bytes.Equal(saved, data) {
			return "", ErrPinnedManifest
		}
		return version, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return "", err
	}
	id, err := NewGeneration()
	if err != nil {
		return "", err
	}
	stage := filepath.Join(bases, ".base-"+string(id))
	defer func() {
		// An invisible stage is retried by the next import if cleanup fails.
		if err := RemoveTree(context.WithoutCancel(ctx), stage); err != nil {
			slog.Warn("machines: " + err.Error())
		}
	}()
	if err := os.MkdirAll(filepath.Join(stage, "rootfs"), 0777); err != nil {
		return "", err
	}
	archive := filepath.Join(stage, manifest.Rootfs.File.Name())
	if err := copyBaseArchive(ctx, filepath.Join(image, manifest.Rootfs.File.Name()), archive, artifacts.Digest{Size: manifest.Rootfs.Size, SHA256: manifest.Rootfs.SHA256}); err != nil {
		return "", err
	}
	system.FaultPoint("base-staged")
	if err := VetArchive(ctx, archive); err != nil {
		return "", err
	}
	deadline := 300 * time.Second
	if _, err := tools.Run(ctx, system.Bsdtar, []string{"-xpf", archive, "--xattrs", "-C", filepath.Join(stage, "rootfs")}, &deadline); err != nil {
		return "", err
	}
	if err := publishBase(ctx, stage, target, data, manifest); err != nil {
		return "", err
	}
	return version, nil
}

// copyBaseArchive keeps the verified archive in the unpublished base stage.
func copyBaseArchive(ctx context.Context, source, destination string, expected artifacts.Digest) (err error) {
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() { _ = from.Close() }() // Read-only release archive.
	to, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0666)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, to.Close()) }()
	if err := artifacts.Copy(ctx, from, expected, to); err != nil {
		var tooLarge *artifacts.TooLargeError
		var size *artifacts.SizeError
		if errors.Is(err, artifacts.ErrDigest) || errors.As(err, &tooLarge) || errors.As(err, &size) {
			return &ArchiveIntegrityError{Source: err}
		}
		return err
	}
	return nil
}

// publishBase checks every embedded executable and durably names the immutable base.
func publishBase(ctx context.Context, stage, target string, data []byte, manifest machinewire.CloudImageManifest) error {
	root, err := os.Open(filepath.Join(stage, "rootfs"))
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }() // Read-only directory handle.
	// Rust's executable map is a BTreeMap; preserve its check order.
	for _, path := range slices.Sorted(maps.Keys(manifest.Executables)) {
		artifact := manifest.Executables[path]
		if err := verifyExecutable(ctx, root, path, artifacts.Digest{Size: artifact.Size, SHA256: artifact.SHA256}); err != nil {
			return err
		}
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	ctx = context.WithoutCancel(ctx)
	if err := os.Remove(filepath.Join(stage, manifest.Rootfs.File.Name())); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(stage, "manifest.json"), data, 0666); err != nil {
		return err
	}
	if err := unix.Syncfs(int(root.Fd())); err != nil {
		return err
	}
	if err := Sync(ctx, stage); err != nil {
		return err
	}
	if err := os.Rename(stage, target); err != nil {
		return err
	}
	return Sync(ctx, filepath.Dir(target))
}

// verifyExecutable opens an image executable beneath the base and verifies its bytes.
func verifyExecutable(ctx context.Context, root *os.File, path string, expected artifacts.Digest) error {
	fd, err := unix.Openat2(int(root.Fd()), strings.TrimLeft(path, "/"), &unix.OpenHow{Flags: unix.O_RDONLY | unix.O_CLOEXEC | unix.O_NOCTTY, Resolve: unix.RESOLVE_BENEATH | unix.RESOLVE_NO_MAGICLINKS})
	if errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
		return &ExecutablePathError{Path: path}
	}
	if err != nil {
		return system.Failed("opening", path, err)
	}
	file := os.NewFile(uintptr(fd), path)
	defer func() { _ = file.Close() }() // Read-only executable.
	info, err := file.Stat()
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return &ExecutableIntegrityError{Path: path}
	}
	if err := artifacts.Copy(ctx, file, expected, io.Discard); err != nil {
		var tooLarge *artifacts.TooLargeError
		var size *artifacts.SizeError
		if errors.Is(err, artifacts.ErrDigest) || errors.As(err, &tooLarge) || errors.As(err, &size) {
			return &ExecutableIntegrityError{Path: path}
		}
		return err
	}
	return nil
}

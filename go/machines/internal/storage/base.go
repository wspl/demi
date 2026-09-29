//go:build linux

package storage

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"log/slog"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/google/uuid"
	"golang.org/x/sys/unix"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

// extractionDeadline is how long bsdtar may take: extraction of a large base can
// take minutes on a slow disk.
const extractionDeadline = 300 * time.Second

// requiredExecutables are the executables every image must embed besides its
// command packages.
var requiredExecutables = [...]string{machinesproto.RunnerPath, machinesproto.InitPath}

// The refusals of a base's import.
var (
	// ErrArchitecture means the image's architecture is not the host's.
	ErrArchitecture = errors.New("Cloud image architecture differs from execution host")
	// ErrDiffers means a base imported under this version holds other manifest
	// bytes.
	ErrDiffers = errors.New("Pinned Cloud image manifest differs")
)

// A MissingExecutableError means the manifest lacks an executable every image
// embeds.
type MissingExecutableError struct {
	Path string
}

func (e *MissingExecutableError) Error() string { return "Cloud image manifest lacks " + e.Path }

// An EscapeError means an executable's path leaves the extracted root through a
// link.
type EscapeError struct {
	Path string
}

func (e *EscapeError) Error() string { return "Invalid image executable path: " + e.Path }

// An ExecutableError means an embedded executable is not the file the manifest
// describes.
type ExecutableError struct {
	Path string
}

func (e *ExecutableError) Error() string { return "Cloud executable integrity mismatch: " + e.Path }

// An IntegrityError means the base archive does not match its manifest.
type IntegrityError struct {
	Err error
}

func (e *IntegrityError) Error() string {
	return "Cloud root archive integrity mismatch: " + e.Err.Error()
}

func (e *IntegrityError) Unwrap() error { return e.Err }

// A release is read and checked, not yet imported.
type release struct {
	manifest *machinesproto.CloudImageManifest
	bytes    []byte
	version  machinesproto.BaseVersion
}

// ImportBase imports the release in image into bases, unless it is there
// already, and returns its base version (docs/cloud/images.md § Import and
// publication): the SHA-256 of the manifest's bytes. A base already imported
// under that version must hold the same manifest bytes.
//
//	<data>/images/bases/<baseVersion>/manifest.json, rootfs/
func ImportBase(ctx context.Context, t *tools.Tools, image, bases string) (machinesproto.BaseVersion, error) {
	prepared, stage, err := prepare(image, bases)
	if err != nil {
		return "", err
	}
	if prepared.manifest == nil {
		return prepared.version, nil
	}
	fault.Point("base-staged")
	err = extract(ctx, t, prepared, stage)
	if err == nil {
		err = publish(prepared, stage, bases)
	}
	if err != nil {
		// The stage is never visible to a runtime; the next import removes it if
		// this removal fails.
		if removed := RemoveTree(stage); removed != nil {
			slog.Warn("machines: " + removed.Error())
		}
		return "", err
	}
	return prepared.version, nil
}

// prepare reads and checks the release, removes stale stages, and copies the
// archive into a new stage, verifying its size and SHA-256 on the way. A release
// that is imported already has no manifest in the result.
func prepare(image, bases string) (release, string, error) {
	data, err := os.ReadFile(filepath.Join(image, "manifest.json"))
	if err != nil {
		return release{}, "", err
	}
	sum := sha256.Sum256(data)
	version := machinesproto.BaseVersion(hex.EncodeToString(sum[:]))
	manifest, err := machinesproto.DecodeManifest(data)
	if err != nil {
		return release{}, "", err
	}
	if host, ok := machinesproto.HostArchitecture(); !ok || host != manifest.Architecture {
		return release{}, "", ErrArchitecture
	}
	for _, required := range requiredExecutables {
		if _, ok := manifest.Executables[required]; !ok {
			return release{}, "", &MissingExecutableError{Path: required}
		}
	}
	if err := CreatePrivate(bases); err != nil {
		return release{}, "", err
	}
	entries, err := os.ReadDir(bases)
	if err != nil {
		return release{}, "", err
	}
	for _, entry := range entries {
		// An import is not visible until its final rename.
		if strings.HasPrefix(entry.Name(), ".base-") {
			if err := RemoveTree(filepath.Join(bases, entry.Name())); err != nil {
				return release{}, "", err
			}
		}
	}
	target := filepath.Join(bases, string(version))
	saved, err := os.ReadFile(filepath.Join(target, "manifest.json"))
	switch {
	case err == nil && slices.Equal(saved, data):
		return release{version: version}, "", nil
	case err == nil:
		return release{}, "", ErrDiffers
	case !errors.Is(err, os.ErrNotExist):
		return release{}, "", err
	}
	stage := filepath.Join(bases, ".base-"+uuid.NewString())
	if err := os.MkdirAll(filepath.Join(stage, "rootfs"), 0o777); err != nil {
		return release{}, "", err
	}
	if err := copyVerified(filepath.Join(image, manifest.Rootfs.File), stage, manifest); err != nil {
		if removed := RemoveTree(stage); removed != nil {
			slog.Warn("machines: " + removed.Error())
		}
		return release{}, "", err
	}
	return release{manifest: manifest, bytes: data, version: version}, stage, nil
}

// copyVerified copies the archive into the stage, checking its size and SHA-256
// as the bytes arrive.
func copyVerified(source, stage string, manifest *machinesproto.CloudImageManifest) error {
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer from.Close()
	to, err := os.OpenFile(filepath.Join(stage, manifest.Rootfs.File), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o666)
	if err != nil {
		return err
	}
	verifier := artifact.NewVerifier(artifact.Digest{Size: manifest.Rootfs.Size, SHA256: manifest.Rootfs.SHA256})
	if _, err := io.Copy(io.MultiWriter(verifier, to), from); err != nil {
		to.Close()
		return &IntegrityError{Err: err}
	}
	if err := to.Close(); err != nil {
		return err
	}
	if err := verifier.Finish(); err != nil {
		return &IntegrityError{Err: err}
	}
	return nil
}

// extract vets the archive's entries, then extracts it with bsdtar, which keeps
// numeric owners, modes with setuid bits, ACLs and extended attributes.
func extract(ctx context.Context, t *tools.Tools, prepared release, stage string) error {
	archive := filepath.Join(stage, prepared.manifest.Rootfs.File)
	if err := VetArchive(archive); err != nil {
		return err
	}
	root := filepath.Join(stage, "rootfs")
	_, err := t.Run(ctx, tools.Bsdtar, []string{"-xpf", archive, "--xattrs", "-C", root}, extractionDeadline)
	return err
}

// publish checks each embedded executable, then makes the stage the base.
func publish(prepared release, stage, bases string) error {
	root, err := os.Open(filepath.Join(stage, "rootfs"))
	if err != nil {
		return err
	}
	defer root.Close()
	for _, path := range slices.Sorted(maps.Keys(prepared.manifest.Executables)) {
		entry := prepared.manifest.Executables[path]
		if err := verifyExecutable(root, path, artifact.Digest{Size: entry.Size, SHA256: entry.SHA256}); err != nil {
			return err
		}
	}
	if err := os.Remove(filepath.Join(stage, prepared.manifest.Rootfs.File)); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(stage, "manifest.json"), prepared.bytes, 0o666); err != nil {
		return err
	}
	if err := unix.Syncfs(int(root.Fd())); err != nil {
		return err
	}
	if err := Sync(stage); err != nil {
		return err
	}
	if err := os.Rename(stage, filepath.Join(bases, string(prepared.version))); err != nil {
		return err
	}
	return Sync(bases)
}

// verifyExecutable opens path beneath the extracted root without following a
// link out of it, and requires a regular file with the expected size and
// SHA-256.
func verifyExecutable(root *os.File, path string, expected artifact.Digest) error {
	relative := strings.TrimLeft(path, "/")
	how := unix.OpenHow{
		Flags:   unix.O_RDONLY | unix.O_CLOEXEC | unix.O_NOCTTY,
		Resolve: unix.RESOLVE_BENEATH | unix.RESOLVE_NO_MAGICLINKS,
	}
	descriptor, err := unix.Openat2(int(root.Fd()), relative, &how)
	switch {
	case err == nil:
	case errors.Is(err, unix.EXDEV), errors.Is(err, unix.ELOOP):
		return &EscapeError{Path: path}
	default:
		return linux.Failed("opening", path, err)
	}
	file := os.NewFile(uintptr(descriptor), path)
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return &ExecutableError{Path: path}
	}
	verifier := artifact.NewVerifier(expected)
	if _, err := io.Copy(verifier, file); err != nil {
		if errors.As(err, new(*artifact.TooLargeError)) {
			return &ExecutableError{Path: path}
		}
		return err
	}
	if err := verifier.Finish(); err != nil {
		return &ExecutableError{Path: path}
	}
	return nil
}

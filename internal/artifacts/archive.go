package artifacts

import (
	"archive/zip"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"
)

const entryBytes = 1024 * 1024 * 1024

// Archive identifies a ZIP and the relative entry its user starts.
type Archive struct {
	// Digest identifies the ZIP bytes verified by the caller.
	Digest Digest
	// Entry is the relative path of the installed executable.
	Entry string
}

// ArchiveError reports an unusable archive.
type ArchiveError struct {
	// Cause is the archive validation or extraction failure.
	Cause error
}

// Error describes why the archive cannot be installed.
func (e *ArchiveError) Error() string {
	return fmt.Sprintf("the archive cannot be installed: %v", e.Cause)
}

// Unwrap returns the archive failure.
func (e *ArchiveError) Unwrap() error { return e.Cause }

// InstallationError reports a corrupt installation that must not be repaired.
type InstallationError struct {
	// Directory identifies the corrupt installation.
	Directory string
	// Reason is the diagnostic fragment describing its failed integrity check.
	Reason string
	// Cause retains the underlying failure when one exists.
	Cause error
}

// Error describes the corrupt installation.
func (e *InstallationError) Error() string {
	return fmt.Sprintf("the installation at %s %s", e.Directory, e.Reason)
}

// Unwrap returns the installation failure.
func (e *InstallationError) Unwrap() error { return e.Cause }

// insideArchive checks the slash-separated paths an artifact may contain.
func insideArchive(name string) bool {
	return name != "." && fs.ValidPath(name) && !strings.ContainsAny(name, "\\:")
}

func validateArchive(archive Archive) error {
	hash, err := hex.DecodeString(archive.Digest.SHA256)
	if err != nil || len(hash) != 32 || strings.ToLower(archive.Digest.SHA256) != archive.Digest.SHA256 ||
		!insideArchive(archive.Entry) {
		return &ArchiveError{errors.New("invalid archive digest or entry path")}
	}
	return nil
}

func readInstallation(ctx context.Context, directory string) (*receipt, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if _, err := os.Stat(directory); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	data, err := ReadReceipt(ctx, directory)
	if err != nil {
		return nil, err
	}
	if data == nil {
		return nil, &InstallationError{Directory: directory, Reason: "has no receipt"}
	}
	found, err := decodeReceipt(data)
	if err != nil {
		return nil, &InstallationError{directory, "has an invalid receipt", err}
	}
	return &found, nil
}

// Recorded trusts the installer's own private cache without rereading its entry.
// An empty path means no installation exists.
func Recorded(ctx context.Context, directory string, archive Archive) (string, error) {
	if err := validateArchive(archive); err != nil {
		return "", err
	}
	found, err := readInstallation(ctx, directory)
	if err != nil || found == nil {
		return "", err
	}
	if found.ArchiveHash != archive.Digest.SHA256 {
		return "", &InstallationError{Directory: directory, Reason: "holds another archive"}
	}
	return artifactPath(directory, filepath.FromSlash(archive.Entry)), nil
}

// Installed checks an installation's receipt and current entry hash. An empty
// path means absent; corrupt installations are errors, never cache misses.
func Installed(ctx context.Context, directory string, archive Archive) (string, error) {
	if err := validateArchive(archive); err != nil {
		return "", err
	}
	found, err := readInstallation(ctx, directory)
	if err != nil || found == nil {
		return "", err
	}
	entry := artifactPath(directory, filepath.FromSlash(archive.Entry))
	digest, err := DigestFile(ctx, entry, entryBytes)
	var tooLarge *TooLargeError
	if errors.Is(err, os.ErrNotExist) || errors.As(err, &tooLarge) {
		return "", &InstallationError{directory, "fails its integrity check", err}
	}
	if err != nil {
		return "", err
	}
	if found.ArchiveHash != archive.Digest.SHA256 || found.EntryHash != digest.SHA256 {
		return "", &InstallationError{Directory: directory, Reason: "fails its integrity check"}
	}
	return entry, nil
}

// Unpacking owns an install lock and its temporary directory. Defer Close as
// soon as InstallArchive returns it, including when writing the ZIP fails.
type Unpacking struct {
	lock                   *InstallLock
	temporary, destination string
	archive                Archive
}

// InstallArchive returns an installed entry or an unpacking operation (exactly
// one is nonempty). Installers of the same digest share a cross-process lock.
func InstallArchive(ctx context.Context, root string, archive Archive) (entry string, unpacking *Unpacking, err error) {
	if err := validateArchive(archive); err != nil {
		return "", nil, err
	}
	if err := ctx.Err(); err != nil {
		return "", nil, err
	}
	if err := os.MkdirAll(root, 0o755); err != nil {
		return "", nil, err
	}
	lock, err := AcquireInstallLock(ctx, artifactPath(root, archive.Digest.SHA256+".lock"))
	if err != nil {
		return "", nil, err
	}
	defer func() {
		if unpacking == nil {
			err = errors.Join(err, lock.Close())
		}
	}()
	destination := artifactPath(root, archive.Digest.SHA256)
	entry, err = Installed(ctx, destination, archive)
	if err != nil || entry != "" {
		return entry, nil, err
	}
	temporary, err := os.MkdirTemp(root, ".install-")
	if err != nil {
		return "", nil, err
	}
	return "", &Unpacking{lock, temporary, destination, archive}, nil
}

// ArchivePath is where the caller writes the ZIP, verifying it during the write.
func (u *Unpacking) ArchivePath() string { return artifactPath(u.temporary, "archive.zip") }

// Close discards staged files and releases the installation lock, idempotently.
func (u *Unpacking) Close() error {
	var err error
	if u.temporary != "" {
		err = removeInstallationStage(u.temporary)
		u.temporary = ""
	}
	if u.lock != nil {
		err = errors.Join(err, u.lock.Close())
		u.lock = nil
	}
	return err
}

// Finish extracts the caller's verified ZIP, records its entry and publishes it.
func (u *Unpacking) Finish(ctx context.Context) (entry string, err error) {
	defer func() { err = errors.Join(err, u.Close()) }()
	if u.temporary == "" {
		return "", os.ErrClosed
	}
	extracted := artifactPath(u.temporary, "extracted")
	if err := extractZIP(ctx, u.ArchivePath(), extracted); err != nil {
		return "", err
	}
	return u.Publish(ctx, extracted)
}

// Publish finishes an installation from an already extracted artifact tree.
// The caller owns extracted until this method moves it into the cache, and
// must remove it on failure. This is the shared publication step used by
// artifactstest.InstallUnpacked; Finish uses it after ZIP extraction.
func (u *Unpacking) Publish(ctx context.Context, extracted string) (entry string, err error) {
	defer func() { err = errors.Join(err, u.Close()) }()
	if u.temporary == "" {
		return "", os.ErrClosed
	}
	return publishInstallation(ctx, extracted, u.destination, u.archive)
}

func publishInstallation(ctx context.Context, extracted, destination string, archive Archive) (string, error) {
	found, err := DigestFile(ctx, artifactPath(extracted, filepath.FromSlash(archive.Entry)), entryBytes)
	if errors.Is(err, os.ErrNotExist) {
		return "", &ArchiveError{fmt.Errorf("holds no %s: %w", archive.Entry, err)}
	}
	if err != nil {
		return "", err
	}
	if err := WriteReceipt(
		ctx,
		extracted,
		receipt{ArchiveHash: archive.Digest.SHA256, EntryHash: found.SHA256},
	); err != nil {
		return "", err
	}
	if err := PublishDirectory(ctx, extracted, destination); err != nil {
		return "", err
	}
	return artifactPath(destination, filepath.FromSlash(archive.Entry)), nil
}

// ZipHolds reads only the directory of an archive to check for a file.
func ZipHolds(ctx context.Context, archive, name string) (bool, error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	reader, err := zip.OpenReader(archive)
	if err != nil {
		return false, &ArchiveError{err}
	}
	defer func() { _ = reader.Close() }() // Read-only ZIP file.
	for _, entry := range reader.File {
		if entry.Name == name {
			return !entry.FileInfo().IsDir(), nil
		}
	}
	return false, nil
}

// extractZIP uses an OS-rooted destination so neither entries nor symlink
// ancestors can cause archive writes to leave the installation directory.
func extractZIP(ctx context.Context, archive, destination string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	reader, err := zip.OpenReader(archive)
	if err != nil {
		return &ArchiveError{err}
	}
	defer func() { _ = reader.Close() }() // Read-only ZIP file.
	if err := os.MkdirAll(destination, 0o755); err != nil {
		return err
	}
	root, err := os.OpenRoot(destination)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }() // Only releases the directory handle.
	directories := make([]*zip.File, 0)
	for _, entry := range reader.File {
		if err := ctx.Err(); err != nil {
			return err
		}
		if err := extractEntry(ctx, root, entry); err != nil {
			return &ArchiveError{err}
		}
		if entry.FileInfo().IsDir() {
			directories = append(directories, entry)
		}
	}
	// Restrictive directory modes are applied only after their children exist,
	// deepest first so an ancestor cannot prevent setting a child's mode.
	sort.Slice(directories, func(i, j int) bool { return len(directories[i].Name) > len(directories[j].Name) })
	for _, entry := range directories {
		if err := ctx.Err(); err != nil {
			return err
		}
		name := filepath.FromSlash(strings.TrimSuffix(entry.Name, "/"))
		if err := root.Chmod(name, entry.Mode().Perm()); err != nil {
			return &ArchiveError{err}
		}
	}
	return nil
}

func extractEntry(ctx context.Context, root *os.Root, entry *zip.File) (err error) {
	name := strings.TrimSuffix(entry.Name, "/")
	if !insideArchive(name) {
		return fmt.Errorf("entry escapes installation: %q", entry.Name)
	}
	if entry.FileInfo().IsDir() {
		return root.MkdirAll(filepath.FromSlash(name), 0o755)
	}
	parent, _ := Parent(filepath.FromSlash(name))
	if parent == "" {
		parent = "."
	}
	if err := root.MkdirAll(parent, 0o755); err != nil {
		return err
	}
	input, err := entry.Open()
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, input.Close()) }()
	if entry.Mode()&os.ModeSymlink != 0 {
		data, err := io.ReadAll(io.LimitReader(input, 4097))
		if err != nil {
			return err
		}
		target := string(data)
		resolved := path.Join(path.Dir(name), target)
		if len(data) > 4096 || path.IsAbs(target) || strings.ContainsAny(target, "\\:") || !insideArchive(resolved) {
			return errors.New("symlink escapes installation")
		}
		return root.Symlink(filepath.FromSlash(target), filepath.FromSlash(name))
	}
	if !entry.Mode().IsRegular() {
		return fmt.Errorf("unsupported archive entry %q", name)
	}
	output, err := root.OpenFile(filepath.FromSlash(name), os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, output.Close()) }()
	if err := transfer(ctx, input, output, nil); err != nil {
		return err
	}
	return output.Chmod(entry.Mode().Perm())
}

// removeInstallationStage removes unpublished artifacts even when their ZIP
// declared directories read-only. Only our private stage is made traversable.
func removeInstallationStage(directory string) error {
	err := os.RemoveAll(directory)
	if !errors.Is(err, fs.ErrPermission) {
		return err
	}
	if err := filepath.WalkDir(directory, func(name string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() {
			return nil
		}
		return os.Chmod(name, 0o700)
	}); err != nil {
		return err
	}
	return os.RemoveAll(directory)
}

package artifact

import (
	"archive/zip"
	"context"
	"errors"
	"io"
	"io/fs"
	"net/http"
	"os"
	"path"
	"path/filepath"
	"strings"
	"unicode/utf8"
)

// executableBytes is the most bytes an installed executable may have: the digest
// of a larger file stops early.
const executableBytes = 1024 * 1024 * 1024

// An Archive is a zip archive to install: the HTTPS URL it is downloaded from,
// the size and SHA-256 it must have, and its executable's path inside it, with
// "/" between the components.
type Archive struct {
	URL        string
	Digest     Digest
	Executable string
}

// Installed returns the executable of the installation of archive at directory,
// once the receipt there names the archive and the executable's current SHA-256,
// or ok false when nothing is at directory. An installation that fails the check
// is an [*InstallationError] rather than a reason to install again or elsewhere:
// its files changed after it was installed.
func Installed(ctx context.Context, directory string, archive Archive) (executable string, ok bool, err error) {
	if _, err := os.Stat(directory); errors.Is(err, fs.ErrNotExist) {
		return "", false, nil
	} else if err != nil {
		return "", false, err
	}
	invalid := func(reason string) error {
		return &InstallationError{Directory: directory, Reason: reason}
	}
	bytes, err := ReadReceipt(directory)
	if err != nil {
		return "", false, err
	}
	if bytes == nil {
		return "", false, invalid("has no receipt")
	}
	receipt, err := decodeReceipt(bytes)
	if err != nil {
		return "", false, invalid("has an invalid receipt: " + err.Error())
	}
	executable = filepath.Join(directory, filepath.FromSlash(archive.Executable))
	found, err := DigestFile(ctx, executable, executableBytes)
	var tooLarge *TooLargeError
	switch {
	case errors.As(err, &tooLarge), errors.Is(err, fs.ErrNotExist):
		return "", false, invalid("fails its integrity check")
	case err != nil:
		return "", false, err
	}
	if receipt.ArchiveHash != archive.Digest.SHA256 || receipt.ExecutableHash != found.SHA256 {
		return "", false, invalid("fails its integrity check")
	}
	return executable, true, nil
}

// InstallArchive installs archive into the directory of root its SHA-256 names
// and returns the executable; an installation already there is checked, not
// replaced. Installers of one archive share the lock <SHA-256>.lock in root, so
// one downloads and the others find its installation.
func InstallArchive(ctx context.Context, client *http.Client, root string, archive Archive) (string, error) {
	sha256 := archive.Digest.SHA256
	if err := os.MkdirAll(root, 0o777); err != nil {
		return "", err
	}
	lock, err := AcquireInstallLock(ctx, filepath.Join(root, sha256+".lock"))
	if err != nil {
		return "", err
	}
	defer lock.Close()
	destination := filepath.Join(root, sha256)
	if executable, ok, err := Installed(ctx, destination, archive); ok || err != nil {
		return executable, err
	}
	// Everything is staged here and gone with it, whatever happens.
	temporary, err := os.MkdirTemp(root, ".install-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(temporary)
	downloaded := filepath.Join(temporary, "archive.zip")
	if err := downloadTo(ctx, client, archive, downloaded); err != nil {
		return "", err
	}
	extracted := filepath.Join(temporary, "extracted")
	if err := extractZip(ctx, downloaded, extracted); err != nil {
		return "", err
	}
	return publishInstallation(ctx, extracted, destination, archive)
}

// downloadTo downloads archive into a new file at path.
func downloadTo(ctx context.Context, client *http.Client, archive Archive, path string) error {
	output, err := os.Create(path)
	if err != nil {
		return err
	}
	defer output.Close()
	return Download(ctx, client, archive.URL, archive.Digest, output)
}

// publishInstallation writes the receipt of archive's files, unpacked at
// extracted, and publishes them as destination; it returns the executable
// there.
func publishInstallation(ctx context.Context, extracted, destination string, archive Archive) (string, error) {
	executable := filepath.FromSlash(archive.Executable)
	found, err := DigestFile(ctx, filepath.Join(extracted, executable), executableBytes)
	if errors.Is(err, fs.ErrNotExist) {
		return "", &ArchiveError{Reason: "holds no " + archive.Executable}
	}
	if err != nil {
		return "", err
	}
	receipt, err := encodeReceipt(archiveReceipt{ArchiveHash: archive.Digest.SHA256, ExecutableHash: found.SHA256})
	if err != nil {
		return "", err
	}
	if err := WriteReceipt(extracted, receipt); err != nil {
		return "", err
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	if err := PublishDirectory(extracted, destination); err != nil {
		return "", err
	}
	return filepath.Join(destination, executable), nil
}

// ZipHolds reports whether the zip archive at archive holds a file at name, such
// as the executable a release record names: a directory or a symbolic link of
// that name is not one. Only the archive's directory is read. A file that cannot
// be opened fails with the operating system's error; one that is not an archive
// with an [*ArchiveError].
func ZipHolds(archive, name string) (bool, error) {
	reader, closer, err := openZip(archive, "read")
	if err != nil {
		return false, err
	}
	defer closer.Close()
	for _, entry := range reader.File {
		if entry.Name == name {
			return !entry.FileInfo().IsDir() && entry.Mode()&fs.ModeSymlink == 0, nil
		}
	}
	return false, nil
}

// openZip opens the zip archive at path. The operating system's failure to open
// the file is returned as it is; a file that is not an archive is an
// [*ArchiveError] that says it cannot be purpose ("read", "extracted").
func openZip(path, purpose string) (*zip.Reader, io.Closer, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, nil, err
	}
	info, err := file.Stat()
	if err != nil {
		file.Close()
		return nil, nil, err
	}
	reader, err := zip.NewReader(file, info.Size())
	if err != nil {
		file.Close()
		return nil, nil, &ArchiveError{Reason: "cannot be " + purpose + ": " + err.Error()}
	}
	return reader, file, nil
}

// extractZip extracts the zip archive at archive into destination. It returns
// only once extraction has stopped, also when ctx is done, so the caller alone
// owns what was extracted. An entry whose path leaves destination is refused,
// and so is a write through a symbolic link that the archive made.
func extractZip(ctx context.Context, archive, destination string) error {
	reader, closer, err := openZip(archive, "extracted")
	if err != nil {
		return err
	}
	defer closer.Close()
	if err := os.MkdirAll(destination, 0o777); err != nil {
		return err
	}
	root, err := os.OpenRoot(destination)
	if err != nil {
		return err
	}
	defer root.Close()
	// The permissions of files are set once every file is written.
	var modes []fileMode
	for _, entry := range reader.File {
		if err := ctx.Err(); err != nil {
			return err
		}
		mode, err := extractEntry(ctx, root, entry)
		if err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return ctxErr
			}
			return unusable(err)
		}
		if mode != nil {
			modes = append(modes, *mode)
		}
	}
	for _, mode := range modes {
		if err := root.Chmod(mode.name, mode.perm); err != nil {
			return unusable(err)
		}
	}
	return nil
}

// unusable words the failure of extracting a zip archive.
func unusable(err error) error {
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return err
	}
	return &ArchiveError{Reason: "cannot be extracted: " + err.Error()}
}

// A fileMode is the permissions the archive gives a file.
type fileMode struct {
	name string
	perm fs.FileMode
}

// creatorUnix is the zip creator system of Unix, whose entries carry
// permissions.
const creatorUnix = 3

// extractEntry writes one entry of the archive below root. It returns the file's
// permissions when the archive gives them.
func extractEntry(ctx context.Context, root *os.Root, entry *zip.File) (*fileMode, error) {
	name := path.Clean(entry.Name)
	if entry.Name == "" || strings.HasPrefix(entry.Name, "/") || !filepath.IsLocal(filepath.FromSlash(name)) {
		return nil, errors.New("invalid file path")
	}
	name = filepath.FromSlash(name)
	if err := refuseLinks(root, name, entry.Mode()&fs.ModeSymlink != 0); err != nil {
		return nil, err
	}
	mode := entry.Mode()
	switch {
	case entry.FileInfo().IsDir():
		return nil, root.MkdirAll(name, 0o777)
	case mode&fs.ModeSymlink != 0:
		return nil, extractSymlink(root, entry, name)
	}
	if err := root.MkdirAll(filepath.Dir(name), 0o777); err != nil {
		return nil, err
	}
	output, err := root.OpenFile(name, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666)
	if err != nil {
		return nil, err
	}
	defer output.Close()
	input, err := entry.Open()
	if err != nil {
		return nil, err
	}
	defer input.Close()
	// Reading to the end checks the entry's CRC.
	if err := copyChunks(ctx, input, output, nil); err != nil {
		return nil, err
	}
	// An entry made without attributes keeps the default permissions.
	if entry.CreatorVersion>>8 != creatorUnix || entry.ExternalAttrs>>16 == 0 {
		return nil, nil
	}
	return &fileMode{name: name, perm: mode.Perm()}, nil
}

// refuseLinks refuses an entry that would write through a symbolic link, inside
// the destination or not: a link stands at a directory of its path, or at its
// whole path when the entry is not itself a link.
func refuseLinks(root *os.Root, name string, isLink bool) error {
	parts := strings.Split(name, string(filepath.Separator))
	if isLink {
		parts = parts[:len(parts)-1]
	}
	for i := range parts {
		prefix := filepath.Join(parts[:i+1]...)
		info, err := root.Lstat(prefix)
		if errors.Is(err, fs.ErrNotExist) {
			return nil
		}
		if err != nil {
			return err
		}
		if info.Mode()&fs.ModeSymlink != 0 {
			return errors.New("would write through a symbolic link")
		}
	}
	return nil
}

// extractSymlink makes the symbolic link an entry stands for: its contents are
// the link's target.
func extractSymlink(root *os.Root, entry *zip.File, name string) error {
	input, err := entry.Open()
	if err != nil {
		return err
	}
	defer input.Close()
	target, err := io.ReadAll(io.LimitReader(input, 4096))
	if err != nil {
		return err
	}
	if !utf8.Valid(target) {
		return errors.New("invalid UTF-8 as symlink target")
	}
	if err := root.MkdirAll(filepath.Dir(name), 0o777); err != nil {
		return err
	}
	return root.Symlink(string(target), name)
}

package claude

import (
	"context"
	"errors"
	"io/fs"
	"maps"
	"os"
	"os/user"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/claudeproto"
)

// binary is the executable's file name inside a version directory.
var binary = func() string {
	if runtime.GOOS == "windows" {
		return "claude.exe"
	}
	return "claude"
}()

// Roots say where installations live.
type Roots struct {
	// Home is the user's root, ~/.demi/claude: installed into, and cleaned of
	// other versions. It is empty when the user has no home directory.
	Home string
	// Image is an image's preinstalled root: used when it has the wanted
	// version, never written.
	Image string
}

// DefaultRoots returns the roots of this machine: the user's home directory's
// .demi/claude (the home directory of the account's entry when the environment
// names none), and, on Unix, /opt/demi/claude.
func DefaultRoots() Roots {
	var roots Roots
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		// The account's entry names a home directory when the environment does not.
		if account, err := user.Current(); err == nil {
			home = account.HomeDir
		}
	}
	if home != "" {
		roots.Home = filepath.Join(home, ".demi", "claude")
	}
	if runtime.GOOS != "windows" {
		roots.Image = "/opt/demi/claude"
	}
	return roots
}

// An Installer holds verified installations of the Claude Code CLI, one directory
// per version. It is safe for concurrent use.
type Installer struct {
	roots Roots
	// allowLoopbackHTTP lets a release name plain HTTP on 127.0.0.1, for a test's
	// fixture server.
	allowLoopbackHTTP bool

	mu sync.Mutex
	// verified holds the executables whose full digest this process has
	// checked, by path and SHA-256.
	verified map[verifiedKey]struct{}
}

type verifiedKey struct {
	path   string
	sha256 string
}

// NewInstaller returns an installer of the roots.
func NewInstaller(roots Roots) *Installer {
	return &Installer{roots: roots, verified: map[verifiedKey]struct{}{}}
}

// Release parses and validates the input of claude.ensure. Its error is an
// [*EnsureError].
func (i *Installer) Release(input []byte) (claudeproto.Release, error) {
	return parseRelease(input, i.allowLoopbackHTTP)
}

// Ensure returns the release's executable for this machine, installing it when
// no usable installation exists. Callers of one version, in this process and
// others, share one download. Every other version under the user's root is
// removed afterwards. It fails with an [*EnsureError], or with the error of ctx
// when it is cancelled.
func (i *Installer) Ensure(ctx context.Context, release claudeproto.Release) (claudeproto.Installed, error) {
	platform, err := currentPlatform()
	if err != nil {
		return claudeproto.Installed{}, err
	}
	build, ok := release.Platforms[platform]
	if !ok {
		return claudeproto.Installed{}, unsupportedPlatform("Claude Code " + release.Version + " has no build for " + platform)
	}
	expected := claudeproto.Receipt{Version: release.Version, Platform: platform, SHA256: build.SHA256, Size: build.Size}
	path, err := i.preinstalled(ctx, expected)
	if err != nil {
		return claudeproto.Installed{}, err
	}
	if path == "" {
		path, err = i.install(ctx, build, expected)
		if err != nil {
			return claudeproto.Installed{}, err
		}
	}
	if home, err := i.home(); err == nil {
		removeOtherVersions(home, release.Version)
	}
	return claudeproto.Installed{Version: expected.Version, Path: path}, nil
}

// Status lists the installations under both roots that have a receipt and an
// executable, newest version first, the image's before the user's for one
// version. Nothing is hashed.
func (i *Installer) Status() (claudeproto.Status, error) {
	platform, err := currentPlatform()
	if err != nil {
		return claudeproto.Status{}, err
	}
	type installation struct {
		version   claudeproto.Version
		installed claudeproto.Installed
	}
	var found []installation
	roots := []string{i.roots.Image}
	if home, err := i.home(); err == nil {
		roots = append(roots, home)
	}
	for _, root := range roots {
		if root == "" {
			continue
		}
		entries, err := os.ReadDir(root)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			name := entry.Name()
			// A directory whose name is not a version is not an installation.
			version, ok := claudeproto.ParseVersion(name)
			if !ok {
				continue
			}
			directory := filepath.Join(root, name)
			path := filepath.Join(directory, binary)
			receipt, ok := readReceipt(directory)
			if !ok || receipt.Version != name {
				continue
			}
			if info, err := os.Stat(path); err != nil || !info.Mode().IsRegular() {
				continue
			}
			found = append(found, installation{version, claudeproto.Installed{Version: name, Path: path}})
		}
	}
	slices.SortFunc(found, func(a, b installation) int {
		if c := b.version.Compare(a.version); c != 0 {
			return c
		}
		return strings.Compare(a.installed.Path, b.installed.Path)
	})
	status := claudeproto.Status{Platform: platform, Installed: []claudeproto.Installed{}}
	for _, one := range found {
		status.Installed = append(status.Installed, one.installed)
	}
	return status, nil
}

func (i *Installer) home() (string, error) {
	home := i.roots.Home
	if home == "" {
		return "", &EnsureError{Code: claudeproto.InstallFailed, Message: "the user's home directory is not set"}
	}
	if !filepath.IsAbs(home) {
		return "", &EnsureError{Code: claudeproto.InstallFailed, Message: "the user's home directory must be absolute"}
	}
	return home, nil
}

// preinstalled returns the executable of the image's installation of the wanted
// version, or "" when the image has none that matches.
func (i *Installer) preinstalled(ctx context.Context, expected claudeproto.Receipt) (string, error) {
	if i.roots.Image == "" {
		return "", nil
	}
	return i.usable(ctx, filepath.Join(i.roots.Image, expected.Version), expected)
}

// install verifies or creates <home>/<version> while holding
// <home>/<version>.lock.
func (i *Installer) install(ctx context.Context, build claudeproto.Artifact, expected claudeproto.Receipt) (string, error) {
	root, err := i.home()
	if err != nil {
		return "", err
	}
	if err := os.MkdirAll(root, 0o777); err != nil {
		return "", installFailed(err)
	}
	lock, err := artifact.AcquireInstallLock(ctx, filepath.Join(root, expected.Version+".lock"))
	if err != nil {
		return "", installFailed(err)
	}
	defer lock.Close()
	destination := filepath.Join(root, expected.Version)
	if path, err := i.usable(ctx, destination, expected); path != "" || err != nil {
		return path, err
	}
	temporary, err := os.MkdirTemp(root, "install-")
	if err != nil {
		return "", installFailed(err)
	}
	defer os.RemoveAll(temporary)
	staged := filepath.Join(temporary, expected.Version)
	if err := os.Mkdir(staged, 0o777); err != nil {
		return "", installFailed(err)
	}
	if err := i.download(ctx, build, expected, filepath.Join(staged, binary)); err != nil {
		return "", err
	}
	receipt, err := claudeproto.Encode(expected)
	if err != nil {
		return "", installFailed(err)
	}
	if err := artifact.WriteReceipt(staged, receipt); err != nil {
		return "", installFailed(err)
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	if err := artifact.PublishDirectory(staged, destination); err != nil {
		return "", installFailed(err)
	}
	path := filepath.Join(destination, binary)
	i.markVerified(path, expected.SHA256)
	return path, nil
}

// download streams the artifact into path, enforcing its size and SHA-256, and
// makes it executable. The caller owns the directory and removes it on any
// failure.
func (i *Installer) download(ctx context.Context, build claudeproto.Artifact, expected claudeproto.Receipt, path string) error {
	client := artifact.NewClient()
	if i.allowLoopbackHTTP {
		client = artifact.NewClientAllowingHTTP()
	}
	output, err := os.Create(path)
	if err != nil {
		return installFailed(err)
	}
	defer output.Close()
	err = artifact.Download(ctx, client, build.URL, artifact.Digest{Size: expected.Size, SHA256: expected.SHA256}, output)
	if err != nil {
		return downloadFailure(err, expected.Version, host(build))
	}
	if err := output.Sync(); err != nil {
		return installFailed(err)
	}
	if err := os.Chmod(path, 0o755); err != nil {
		return installFailed(err)
	}
	return nil
}

// downloadFailure says what a failure of a download means for the version from
// host: a download that failed, or bytes that are not the declared ones.
func downloadFailure(err error, version, host string) error {
	var (
		download *artifact.DownloadError
		rejected *artifact.RejectedError
		large    *artifact.TooLargeError
		size     *artifact.SizeError
	)
	switch {
	case isCancellation(err):
		return err
	case errors.As(err, &download), errors.As(err, &rejected):
		return downloadFailed("Claude Code " + version + " download from " + host + " failed: " + err.Error())
	case errors.As(err, &large), errors.As(err, &size), errors.Is(err, artifact.ErrDigest):
		return verificationFailed("Claude Code " + version + " from " + host + ": " + err.Error())
	}
	return installFailed(err)
}

// usable returns the executable in directory when its receipt equals expected,
// its size matches and its SHA-256 matches, or "" when it is not. The digest is
// computed once per process for each executable; later calls compare receipt and
// size only.
func (i *Installer) usable(ctx context.Context, directory string, expected claudeproto.Receipt) (string, error) {
	if receipt, ok := readReceipt(directory); !ok || receipt != expected {
		return "", nil
	}
	path := filepath.Join(directory, binary)
	info, err := os.Stat(path)
	if err != nil || !info.Mode().IsRegular() || uint64(info.Size()) != expected.Size {
		return "", nil
	}
	if i.isVerified(path, expected.SHA256) {
		return path, nil
	}
	digest, err := artifact.DigestFile(ctx, path, expected.Size)
	switch {
	case isCancellation(err):
		return "", err
	case err != nil || digest.SHA256 != expected.SHA256:
		return "", nil
	}
	i.markVerified(path, expected.SHA256)
	return path, nil
}

func (i *Installer) isVerified(path, sha256 string) bool {
	i.mu.Lock()
	defer i.mu.Unlock()
	_, ok := i.verified[verifiedKey{path, sha256}]
	return ok
}

func (i *Installer) markVerified(path, sha256 string) {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.verified[verifiedKey{path, sha256}] = struct{}{}
}

func currentPlatform() (string, error) {
	platform, ok := CurrentPlatform()
	if !ok {
		return "", unsupportedPlatform("Claude Code has no build for this machine (" + runtime.GOOS + " " + runtime.GOARCH + ")")
	}
	return platform, nil
}

// readReceipt returns the receipt in directory; it reports false when there is
// none or one that does not decode, which makes the installation unusable.
func readReceipt(directory string) (claudeproto.Receipt, bool) {
	bytes, err := artifact.ReadReceipt(directory)
	if err != nil || bytes == nil {
		return claudeproto.Receipt{}, false
	}
	receipt, err := claudeproto.Decode[claudeproto.Receipt](bytes)
	return receipt, err == nil
}

// removeOtherVersions removes every other version directory. A failure leaves
// that version for a later call: Windows cannot remove an executable that is
// running.
func removeOtherVersions(root, keep string) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	for _, name := range slices.Sorted(maps.Keys(directories(entries))) {
		if name != keep && claudeproto.IsVersion(name) {
			// The version stays for a later call when it cannot be removed.
			os.RemoveAll(filepath.Join(root, name))
		}
	}
}

// directories returns the names of the directories among entries.
func directories(entries []fs.DirEntry) map[string]struct{} {
	names := map[string]struct{}{}
	for _, entry := range entries {
		if entry.IsDir() {
			names[entry.Name()] = struct{}{}
		}
	}
	return names
}

package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"

	"github.com/klauspost/compress/zstd"
	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// The generated-code check covers contractgen, not the embedded uv pin copy.
func TestEmbeddedUVPinMatchesSource(t *testing.T) {
	source := readFixture(t, filepath.Join("..", "..", "cloud-guest-image", "rootfs", "uv.json"))
	if !bytes.Equal(uvPin, source) {
		t.Fatal("tools/release/uv.json is stale; run CGO_ENABLED=0 GOFLAGS=-mod=readonly go generate ./tools/release from the repository root")
	}
}

type imageFixture struct {
	app     *application
	options imageOptions
	pin     uvRelease
	client  *artifacts.Client
	server  *artifactstest.Server
}

func newImageFixture(t *testing.T) *imageFixture {
	t.Helper()
	a := appFixture(t)
	target := commandwire.Targets[2]
	runners := filepath.Join(a.Root, "runners")
	nativeFixture(t, a.Root, "demi-runner", []string{target}, "runner")
	if err := a.packageNative(t.Context(), packageOptions{Package: "demi-runner", Output: runners, Targets: []string{target}}); err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(a.Root, "rootfs")
	writeFixture(t, inTree(root, "/var/lib/dpkg/status"), []byte("Package: base-files\nStatus: install ok installed\nVersion: 14ubuntu1\nDescription: ignored\n continued\n\nPackage: tini\nStatus: install ok installed\nVersion: 0.19.0-3\n\nPackage: removed\nStatus: deinstall ok config-files\nVersion: 1\n"))
	writeFixture(t, inTree(root, "/usr/lib/os-release"), []byte("ID=ubuntu\nVERSION_ID=\"26.04\"\n"))
	writeFixture(t, inTree(root, machinewire.InitPath), []byte("tini"))
	if err := os.MkdirAll(inTree(root, "/usr/local/bin"), 0755); err != nil {
		t.Fatal(err)
	}
	chrome := artifactstest.Zip(t, map[string][]byte{"chrome/chrome": []byte("Chrome")})
	archivePath := filepath.Join(a.Root, "archive")
	writeFixture(t, archivePath, chrome)
	digest, err := measureExecutable(t.Context(), archivePath)
	if err != nil {
		t.Fatal(err)
	}
	writeFixture(t, filepath.Join(a.Root, ".cache/resources", digest.SHA256), chrome)
	program := filepath.Join(a.Root, "browser")
	writeFixture(t, program, []byte("browser"))
	release := filepath.Join(a.Root, "browser-release")
	pin := browserop.BrowserRelease{Version: "153.0.8010.36", Platforms: []browserop.ReleasePlatform{{Target: target, URL: "https://example.test/chrome.zip", Size: digest.Size, SHA256: digest.SHA256, Executable: "chrome/chrome"}}}
	if err := a.publishNative(t.Context(), packageOptions{Package: "demi-browser", Output: release}, map[string]string{target: program}, pin); err != nil {
		t.Fatal(err)
	}

	claude := filepath.Join(a.Root, "claude-release")
	nativeFixture(t, a.Root, "demi-claude-code", []string{target}, "claude")
	if err := a.packageNative(t.Context(), packageOptions{Package: "demi-claude-code", Output: claude, Targets: []string{target}}); err != nil {
		t.Fatal(err)
	}
	var compressed bytes.Buffer
	gzipWriter := gzip.NewWriter(&compressed)
	writer := tar.NewWriter(gzipWriter)
	for _, name := range []string{"uv/uv", "uv/uvx"} {
		if err := writer.WriteHeader(&tar.Header{Name: name, Mode: 0755, Size: int64(len(name))}); err != nil {
			t.Fatal(err)
		}
		if _, err := io.WriteString(writer, name); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gzipWriter.Close(); err != nil {
		t.Fatal(err)
	}
	server := artifactstest.Start(t, map[string]artifactstest.Answer{"/uv.tar.gz": artifactstest.OK(compressed.Bytes())})
	path := filepath.Join(a.Root, "uv.tar.gz")
	writeFixture(t, path, compressed.Bytes())
	uv, err := measureExecutable(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	client := artifacts.NewClientAllowingHTTP()
	t.Cleanup(client.Close)
	uvPin := uvRelease{Version: "0.12.13", ARM64: uvArchive{URL: server.URL("/uv.tar.gz"), Size: uv.Size, SHA256: uv.SHA256, Executables: []string{"uv/uv", "uv/uvx"}}}
	return &imageFixture{a, imageOptions{Root: root, Runners: runners, Packages: []string{release, claude}, Output: filepath.Join(a.Root, "image")}, uvPin, client, server}
}

// fixtureImageArchive substitutes only the Linux host's GNU tar process. The
// scenario still packages real files, installs resources and validates the exact
// manifest through the manager's decoder. GNU tar metadata requires Linux acceptance.
func fixtureImageArchive(ctx context.Context, root, path string) (err error) {
	file, err := os.Create(path)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, file.Close()) }()
	encoder, err := zstd.NewWriter(file)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, encoder.Close()) }()
	writer := tar.NewWriter(encoder)
	defer func() { err = errors.Join(err, writer.Close()) }()
	return filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		if path == root {
			return nil
		}
		link := ""
		if info.Mode()&os.ModeSymlink != 0 {
			link, err = os.Readlink(path)
			if err != nil {
				return err
			}
		}
		header, err := tar.FileInfoHeader(info, link)
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		header.Name = filepath.ToSlash(relative)
		if err := writer.WriteHeader(header); err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		_, err = writer.Write(data)
		return err
	})
}
func TestImageEmbedsVerifiedInputsAndPublishesManagerManifest(t *testing.T) {
	f := newImageFixture(t)
	var output bytes.Buffer
	f.app.Out = &output
	writer := fixtureImageArchive
	if runtime.GOOS == "linux" {
		writer = f.app.writeArchive
	}
	if err := f.app.packageImage(t.Context(), f.options, machinewire.ArchitectureARM64, f.pin, f.client, writer); err != nil {
		t.Fatal(err)
	}
	manifestPath := filepath.Join(f.options.Output, "manifest.json")
	manifest, err := machinewire.DecodeCloudImageManifest(readFixture(t, manifestPath))
	if err != nil {
		t.Fatal(err)
	}

	if manifest.FormatVersion != 1 || manifest.OS != machinewire.OSLinux || manifest.Architecture != machinewire.ArchitectureARM64 || manifest.Ubuntu != "26.04" {
		t.Fatal(manifest)
	}
	if !reflect.DeepEqual(manifest.Packages, []machinewire.InstalledPackage{{Name: "base-files", Version: "14ubuntu1"}, {Name: "tini", Version: "0.19.0-3"}}) {
		t.Fatal(manifest.Packages)
	}
	if !reflect.DeepEqual(manifest.Runner, readRunner(t, f.options.Runners)) {
		t.Fatal("runner release differs")
	}
	var releases []commandwire.PackageDescriptor
	expectedExecutables := map[string]commandwire.PackageArtifact{}
	for path, data := range map[string][]byte{
		machinewire.RunnerPath: []byte("runner " + commandwire.Targets[2]),
		machinewire.InitPath:   []byte("tini"),
		"/usr/local/bin/uv":    []byte("uv/uv"),
		"/usr/local/bin/uvx":   []byte("uv/uvx"),
	} {
		source := filepath.Join(t.TempDir(), "source")
		writeFixture(t, source, data)
		digest, err := measureExecutable(t.Context(), source)
		if err != nil {
			t.Fatal(err)
		}
		expectedExecutables[path] = digest
	}
	for i, directory := range f.options.Packages {
		descriptor, err := commandwire.DecodePackageDescriptor(readFixture(t, filepath.Join(directory, "descriptor.json")))
		if err != nil {
			t.Fatal(err)
		}
		releases = append(releases, descriptor)
		program := []string{"demi-browser", "demi-claude-code"}[i]
		source := []byte("browser")
		if i == 1 {
			source = []byte("claude " + commandwire.Targets[2])
		}
		sum := sha256.Sum256(source)
		artifact := commandwire.PackageArtifact{SHA256: hex.EncodeToString(sum[:]), Size: uint64(len(source))}
		expectedExecutables[runnerwire.ArtifactsPath+"/"+artifact.SHA256+"/"+program] = artifact
	}
	resource := releases[0].Resources["chrome"].Targets[commandwire.Targets[2]]
	chromeSource := filepath.Join(t.TempDir(), "chrome")
	writeFixture(t, chromeSource, []byte("Chrome"))
	chromeDigest, err := measureExecutable(t.Context(), chromeSource)
	if err != nil {
		t.Fatal(err)
	}
	expectedExecutables[runnerwire.ArtifactsPath+"/"+resource.SHA256+"/"+resource.Entry] = chromeDigest
	if !reflect.DeepEqual(manifest.Executables, expectedExecutables) {
		t.Fatalf("executables: %v, want %v", manifest.Executables, expectedExecutables)
	}
	if !reflect.DeepEqual(manifest.Releases, releases) {
		t.Fatal("package releases differ")
	}
	if !reflect.DeepEqual(manifest.Tools, []machinewire.StandaloneTool{{Name: "uv", Version: f.pin.Version, SHA256: f.pin.ARM64.SHA256}}) {
		t.Fatal(manifest.Tools)
	}
	requireReleaseEntries(t, f.options.Output, "manifest.json", "rootfs.tar.zst")

	digest, err := measureExecutable(t.Context(), manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(output.String()) != digest.SHA256 {
		t.Fatal("base version does not hash published manifest")
	}
	archive := readFixture(t, filepath.Join(f.options.Output, string(manifest.Rootfs.File)))
	verifier := artifacts.NewVerifier(artifacts.Digest{Size: manifest.Rootfs.Size, SHA256: manifest.Rootfs.SHA256})
	if err := verifier.Update(archive); err != nil {
		t.Fatal(err)
	}
	if err := verifier.Finish(); err != nil {
		t.Fatal(err)
	}
	reader, err := zstd.NewReader(bytes.NewReader(archive))
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	tarReader := tar.NewReader(reader)
	found := map[string][]byte{}
	alias := ""
	for {
		header, err := tarReader.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			t.Fatal(err)
		}

		name := strings.TrimPrefix(header.Name, "./")
		if strings.HasSuffix(name, ".lock") {
			t.Fatalf("archive contains lock %s", name)
		}
		switch header.Typeflag {
		case tar.TypeDir, tar.TypeReg, tar.TypeSymlink:
		default:
			t.Fatalf("unexpected archive kind %d for %s", header.Typeflag, name)
		}
		if name == "usr/bin/demi" {
			if header.Typeflag != tar.TypeSymlink {
				t.Fatal("runner alias is not a symlink")
			}
			alias = header.Linkname
		}
		if _, executable := expectedExecutables["/"+name]; executable && header.Typeflag != tar.TypeReg {
			t.Fatalf("executable is not regular: %s", name)
		}

		data, err := io.ReadAll(tarReader)
		if err != nil {
			t.Fatal(err)
		}
		found["/"+name] = data
	}
	if alias != "demi-runner" {
		t.Fatal("runner alias missing")
	}
	for path, expected := range manifest.Executables {
		data, ok := found[path]
		if !ok {
			t.Fatalf("archive misses %s", path)
		}
		verifier := artifacts.NewVerifier(artifacts.Digest{Size: expected.Size, SHA256: expected.SHA256})
		if err := verifier.Update(data); err != nil {
			t.Fatal(err)
		}
		if err := verifier.Finish(); err != nil {
			t.Fatal(path, err)
		}
	}
	if _, ok := found[runnerwire.ArtifactsPath+"/"+resource.SHA256+"/"+artifacts.ReceiptFile]; !ok {
		t.Fatal("resource receipt missing from archive")
	}
	entry, err := artifacts.Installed(t.Context(), filepath.Join(inTree(f.options.Root, runnerwire.ArtifactsPath), resource.SHA256), artifacts.Archive{Digest: artifacts.Digest{Size: resource.Size, SHA256: resource.SHA256}, Entry: resource.Entry})
	if err != nil || entry == "" {
		t.Fatalf("runner cannot reuse embedded resource: %s, %v", entry, err)
	}
	if f.server.Requests() != 1 {
		t.Fatal("uv downloaded more than once")
	}
}
func TestCorruptArtifactOrUnfinishedPackagePublishesNoImage(t *testing.T) {
	for _, scenario := range []string{"runner", "command", "resource", "uv", "dpkg", "duplicate package", "missing target", "tini symlink", "archive failure"} {
		t.Run(scenario, func(t *testing.T) {
			f := newImageFixture(t)
			target := commandwire.Targets[2]
			writer := fixtureImageArchive
			switch scenario {
			case "runner":
				release := readRunner(t, f.options.Runners)
				writeFixture(t, filepath.Join(f.options.Runners, release.Release, target, "demi-runner"), []byte("wrong"))
			case "command":
				writeFixture(t, filepath.Join(f.options.Packages[0], target, "demi-browser"), []byte("BROWSER"))
			case "resource":
				descriptor, err := commandwire.DecodePackageDescriptor(readFixture(t, filepath.Join(f.options.Packages[0], "descriptor.json")))
				if err != nil {
					t.Fatal(err)
				}
				resource := descriptor.Resources["chrome"].Targets[target]
				writeFixture(t, filepath.Join(f.options.Packages[0], "resources", resource.SHA256), []byte("wrong"))
			case "uv":
				f.pin.ARM64.SHA256 = strings.Repeat("0", 64)
			case "dpkg":
				writeFixture(t, inTree(f.options.Root, "/var/lib/dpkg/status"), []byte("Package: tini\nStatus: install ok half-configured\nVersion: 1\n"))
			case "duplicate package":
				f.options.Packages = append(f.options.Packages, f.options.Packages[0])
			case "missing target":
				path := filepath.Join(f.options.Packages[0], "descriptor.json")
				descriptor, err := commandwire.DecodePackageDescriptor(readFixture(t, path))
				if err != nil {
					t.Fatal(err)
				}
				descriptor.Targets[commandwire.Targets[0]] = descriptor.Targets[target]
				delete(descriptor.Targets, target)
				data, err := record(descriptor)
				if err != nil {
					t.Fatal(err)
				}
				writeFixture(t, path, data)
			case "tini symlink":
				path := inTree(f.options.Root, machinewire.InitPath)
				if err := os.Remove(path); err != nil {
					t.Fatal(err)
				}
				if err := os.Symlink("demi-runner", path); err != nil {
					t.Fatal(err)
				}
			case "archive failure":
				writer = func(context.Context, string, string) error { return io.ErrUnexpectedEOF }
			}

			err := f.app.packageImage(t.Context(), f.options, machinewire.ArchitectureARM64, f.pin, f.client, writer)
			if err == nil {
				t.Fatal("bad image accepted")
			}
			if scenario == "command" && (!errors.Is(err, artifacts.ErrDigest) || !strings.Contains(err.Error(), "demi-browser")) {
				t.Fatalf("command corruption: %v", err)
			}
			if scenario == "dpkg" && err.Error() != `dpkg lists tini as "install ok half-configured": its installation did not finish` {
				t.Fatalf("unfinished package unnamed: %v", err)
			}

			if _, err := os.Stat(f.options.Output); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("published failed image: %v", err)
			}
			stages, err := filepath.Glob(filepath.Join(f.app.Root, ".cloud-image-*"))
			if err != nil || len(stages) != 0 {
				t.Fatalf("stage leaked: %v %v", stages, err)
			}
			if (scenario == "dpkg" || scenario == "duplicate package" || scenario == "missing target") && f.server.Requests() != 0 {
				t.Fatal("invalid input downloaded uv")
			}
		})
	}
}

package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
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
	target := commandwire.Targets[3]
	runners := filepath.Join(a.Root, "runners")
	nativeFixture(t, a.Root, "demi-runner", []string{target}, "runner")
	if err := a.packageNative(t.Context(), packageOptions{Package: "demi-runner", Output: runners, Targets: []string{target}}); err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(a.Root, "rootfs")
	writeFixture(t, inTree(root, "/var/lib/dpkg/status"), []byte("Package: bash\nStatus: install ok installed\nVersion: 1.2\nDescription: ignored\n continued\n\nPackage: removed\nStatus: deinstall ok config-files\nVersion: 1\n"))
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
	uvPin := uvRelease{Version: "0.12.13", AMD64: uvArchive{URL: server.URL("/uv.tar.gz"), Size: uv.Size, SHA256: uv.SHA256, Executables: []string{"uv/uv", "uv/uvx"}}}
	return &imageFixture{a, imageOptions{Root: root, Runners: runners, Packages: []string{release}, Output: filepath.Join(a.Root, "image")}, uvPin, client, server}
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
	if err := f.app.packageImage(t.Context(), f.options, machinewire.ArchitectureAMD64, f.pin, f.client, fixtureImageArchive); err != nil {
		t.Fatal(err)
	}
	manifestPath := filepath.Join(f.options.Output, "manifest.json")
	manifest, err := machinewire.DecodeCloudImageManifest(readFixture(t, manifestPath))
	if err != nil {
		t.Fatal(err)
	}
	if manifest.Ubuntu != "26.04" || len(manifest.Packages) != 1 || manifest.Packages[0].Name != "bash" || len(manifest.Releases) != 1 || len(manifest.Tools) != 1 || manifest.Tools[0].Version != f.pin.Version {
		t.Fatal(manifest)
	}
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
		if header.Name == "usr/bin/demi" {
			alias = header.Linkname
		}
		data, err := io.ReadAll(tarReader)
		if err != nil {
			t.Fatal(err)
		}
		found["/"+header.Name] = data
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
	resource := manifest.Releases[0].Resources["chrome"].Targets[commandwire.Targets[3]]
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
			target := commandwire.Targets[3]
			writer := fixtureImageArchive
			switch scenario {
			case "runner":
				release := readRunner(t, f.options.Runners)
				writeFixture(t, filepath.Join(f.options.Runners, release.Release, target, "demi-runner"), []byte("wrong"))
			case "command":
				writeFixture(t, filepath.Join(f.options.Packages[0], target, "demi-browser"), []byte("wrong"))
			case "resource":
				descriptor, err := commandwire.DecodePackageDescriptor(readFixture(t, filepath.Join(f.options.Packages[0], "descriptor.json")))
				if err != nil {
					t.Fatal(err)
				}
				resource := descriptor.Resources["chrome"].Targets[target]
				writeFixture(t, filepath.Join(f.options.Packages[0], "resources", resource.SHA256), []byte("wrong"))
			case "uv":
				f.pin.AMD64.SHA256 = strings.Repeat("0", 64)
			case "dpkg":
				writeFixture(t, inTree(f.options.Root, "/var/lib/dpkg/status"), []byte("Package: bash\nStatus: install ok unpacked\nVersion: 1\n"))
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
			if err := f.app.packageImage(t.Context(), f.options, machinewire.ArchitectureAMD64, f.pin, f.client, writer); err == nil {
				t.Fatal("bad image accepted")
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

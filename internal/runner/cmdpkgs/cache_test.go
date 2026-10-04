package cmdpkgs_test

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runnerproto"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	if os.Getenv("DEMI_STARTUP_PEER") != "" {
		startupPeerMain()
	}
	code := programtest.Run(m)
	if err := goleak.Find(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		code = 1
	}
	os.Exit(code)
}

type localResolver struct {
	path      string
	calls     atomic.Int32
	entered   chan struct{}
	cancelled chan struct{}
	gate      <-chan struct{}
}

func (r *localResolver) Resolve(
	ctx context.Context,
	_ cmdproto.PackageArtifact,
) (cmdpkgs.ArtifactSource, error) {
	defer func() {
		if ctx.Err() != nil && r.cancelled != nil {
			r.cancelled <- struct{}{}
		}
	}()
	r.calls.Add(1)
	if r.entered != nil {
		select {
		case r.entered <- struct{}{}:
		case <-ctx.Done():
			return cmdpkgs.ArtifactSource{}, ctx.Err()
		}
	}
	if r.gate != nil {
		select {
		case <-r.gate:
		case <-ctx.Done():
			return cmdpkgs.ArtifactSource{}, ctx.Err()
		}
	}
	return cmdpkgs.ArtifactSource{Path: r.path}, nil
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func artifact(data []byte) cmdproto.PackageArtifact {
	return cmdproto.PackageArtifact{SHA256: fmt.Sprintf("%x", sha256.Sum256(data)), Size: uint64(len(data))}
}

func wanted(name, version string, data []byte) cmdpkgs.Wanted {
	return cmdpkgs.Wanted{
		Package:  "demi.fixture",
		Name:     name,
		Version:  version,
		Artifact: artifact(data),
		Form:     &cmdproto.ArtifactFile{},
	}
}

func newCache(t *testing.T) (*cmdpkgs.ArtifactCache, *cmdpkgs.Installs, string) {
	t.Helper()
	root := t.TempDir()
	installs := &cmdpkgs.Installs{}
	cache, err := cmdpkgs.NewArtifactCache(t.Context(), filepath.Join(root, "cache"), "", installs)
	must(t, err)
	t.Cleanup(func() {
		must(t, cache.Close(context.Background()))
		installs.Close()
	})
	return cache, installs, root
}

func writeSource(t *testing.T, root, name string, data []byte) *localResolver {
	t.Helper()
	path := filepath.Join(root, name)
	must(t, os.WriteFile(path, data, 0o700))
	return &localResolver{path: path}
}

func TestCachedFileIsReusedAndWrongSizeFails(t *testing.T) {
	cache, _, root := newCache(t)
	data := []byte("native executable fixture")
	resolver := writeSource(t, root, "source", data)
	w := wanted("program", "1.0.0", data)
	path, err := cache.Install(t.Context(), w, resolver)
	must(t, err)
	got, err := os.ReadFile(path)
	must(t, err)
	if !bytes.Equal(got, data) {
		t.Fatalf("bytes = %q", got)
	}
	info, err := os.Stat(path)
	must(t, err)
	if runtime.GOOS != "windows" && info.Mode().Perm()&0o111 != 0o111 {
		t.Fatal("cached file is not executable")
	}
	again, err := cache.Install(t.Context(), w, resolver)
	must(t, err)
	if again != path || resolver.calls.Load() != 1 {
		t.Fatal("cache hit asked resolver")
	}
	must(t, os.WriteFile(path, []byte("corrupt cache"), 0o700))
	_, err = cache.Install(t.Context(), w, resolver)
	var size *artifacts.SizeError
	if !errors.As(err, &size) {
		t.Fatalf("wrong size accepted: %v", err)
	}
}

func TestMismatchedOrCancelledInstallLeavesNothing(t *testing.T) {
	cache, _, root := newCache(t)
	resolver := writeSource(t, root, "source", []byte("too much data"))
	_, err := cache.Install(t.Context(), wanted("program", "1", []byte("small")), resolver)
	var failure *cmdpkgs.RuntimeError
	if !errors.As(err, &failure) || failure.Kind != cmdpkgs.ArtifactFailure {
		t.Fatalf("mismatch: %v", err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	_, err = cache.Install(ctx, wanted("program", "1", []byte("too much data")), resolver)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("cancellation: %v", err)
	}
	entries, err := os.ReadDir(filepath.Join(root, "cache"))
	must(t, err)
	if len(entries) != 0 {
		t.Fatalf("left entries: %v", entries)
	}
}

func TestArchiveUnpackedOnceAndReported(t *testing.T) {
	cache, installs, root := newCache(t)
	var data bytes.Buffer
	archive := zip.NewWriter(&data)
	for _, entry := range []struct{ name, contents string }{
		{"chrome-linux64/chrome", "chrome"},
		{"chrome-linux64/LICENSE", "license"},
	} {
		writer, err := archive.Create(entry.name)
		must(t, err)
		_, err = writer.Write([]byte(entry.contents))
		must(t, err)
	}
	must(t, archive.Close())
	resolver := writeSource(t, root, "chrome.zip", data.Bytes())
	gate := make(chan struct{})
	resolver.gate = gate
	reported := installs.Subscribe()
	w := wanted("Chrome for Testing", "153.0.8010.36", data.Bytes())
	w.Package = "demi.browser"
	w.Form = &cmdproto.ArtifactArchive{Entry: "chrome-linux64/chrome"}
	type result struct {
		path string
		err  error
	}
	done := make(chan result, 1)
	go func() {
		path, err := cache.Install(t.Context(), w, resolver)
		done <- result{path, err}
	}()
	t.Cleanup(func() {
		select {
		case <-gate:
		default:
			close(gate)
		}
	})
	for len(reported.Current()) == 0 {
		_, err := reported.Changed(t.Context())
		must(t, err)
	}
	expected := []runnerproto.Install{
		{
			Package: w.Package,
			Name:    w.Name,
			Version: w.Version,
			Phase:   runnerproto.InstallPhaseDownload,
			Total:   w.Artifact.Size,
		},
	}
	if got := reported.Current(); !reflect.DeepEqual(got, expected) {
		t.Fatalf("installs = %#v", got)
	}
	close(gate)
	installed := <-done
	must(t, installed.err)
	expectedPath := filepath.Join(root, "cache", w.Artifact.SHA256, "chrome-linux64/chrome")
	if installed.path != expectedPath {
		t.Fatalf("path = %s", installed.path)
	}
	got, err := os.ReadFile(installed.path)
	must(t, err)
	if string(got) != "chrome" || len(reported.Current()) != 0 {
		t.Fatal("archive contents/progress wrong")
	}
	again, err := cache.Install(t.Context(), w, resolver)
	must(t, err)
	if again != installed.path || resolver.calls.Load() != 1 {
		t.Fatal("archive not reused")
	}
}

func TestNewerVersionRemovesOnlyUnheldArtifactsOfItsLine(t *testing.T) {
	cache, _, root := newCache(t)
	install := func(name, version string) (string, cmdpkgs.Wanted) {
		t.Helper()
		data := []byte(name + " " + version)
		resolver := writeSource(t, root, name+version, data)
		w := wanted(name, version, data)
		path, err := cache.Install(t.Context(), w, resolver)
		must(t, err)
		return path, w
	}
	first, _ := install("cli", "1")
	second, two := install("cli", "2")
	other, _ := install("program", "0.1.3")
	if _, err := os.Stat(first); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("old version remains")
	}
	initial, err := cache.Installed(t.Context(), "demi.fixture", "cli")
	must(t, err)
	if len(initial) != 1 || initial[0].Version != "2" {
		t.Fatalf("initial versions = %v", initial)
	}
	if _, err := os.Stat(other); err != nil {
		t.Fatal(err)
	}
	hold := cache.Holds().Hold(two.Artifact.SHA256)
	defer hold.Release()
	_, three := install("cli", "3")
	listed, err := cache.Installed(t.Context(), "demi.fixture", "cli")
	must(t, err)
	versions := make([]string, 0, len(listed))
	for _, entry := range listed {
		versions = append(versions, entry.Version)
	}
	if !reflect.DeepEqual(versions, []string{"3", "2"}) {
		t.Fatalf("versions = %v", versions)
	}
	_, err = os.Stat(second)
	must(t, err)
	_, err = os.Stat(other)
	must(t, err)
	hold.Release()
	_, err = cache.Install(t.Context(), three, &localResolver{path: filepath.Join(root, "cli3")})
	must(t, err)
	if _, err := os.Stat(second); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("released old version remains")
	}
}

func TestImageArchiveWithMissingEntryLogsAndDownloads(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Cloud images are Linux-only; Windows does not inspect them")
	}
	root := t.TempDir()
	image := filepath.Join(root, "image")
	data := artifactstest.Zip(t, map[string][]byte{"bin/tool": []byte("tool")})
	w := wanted("tool", "1", data)
	w.Form = &cmdproto.ArtifactArchive{Entry: "bin/tool"}
	archive := artifacts.Archive{
		Digest: artifacts.Digest{SHA256: w.Artifact.SHA256, Size: w.Artifact.Size},
		Entry:  "bin/tool",
	}
	_, unpacking, err := artifacts.InstallArchive(t.Context(), image, archive)
	must(t, err)
	t.Cleanup(func() {
		must(t, unpacking.Close())
	})
	must(t, os.WriteFile(unpacking.ArchivePath(), data, 0o600))
	entry, err := unpacking.Finish(t.Context())
	must(t, err)
	must(t, os.Remove(entry))

	logs := &logCapture{}
	previous := slog.Default()
	slog.SetDefault(slog.New(logs))
	defer slog.SetDefault(previous)
	installs := &cmdpkgs.Installs{}
	t.Cleanup(installs.Close)
	cache, err := cmdpkgs.NewArtifactCache(t.Context(), filepath.Join(root, "cache"), image, installs)
	must(t, err)
	t.Cleanup(func() {
		must(t, cache.Close(context.Background()))
	})
	resolver := writeSource(t, root, "source.zip", data)
	got, err := cache.Install(t.Context(), w, resolver)
	must(t, err)
	want := filepath.Join(root, "cache", w.Artifact.SHA256, "bin/tool")
	if got != want || resolver.calls.Load() != 1 {
		t.Fatalf("installation = %q, downloads = %d; want %q, 1", got, resolver.calls.Load(), want)
	}
	if _, err := os.Stat(entry); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("image entry stat = %v; want not exist", err)
	}
	logs.mu.Lock()
	defer logs.mu.Unlock()
	message := "preinstalled artifact in " + filepath.Join(image, w.Artifact.SHA256) +
		" not used, downloading it: the installation at " + filepath.Join(image, w.Artifact.SHA256) +
		" fails its integrity check"
	if len(logs.lines) != 1 || logs.lines[0].Message != message {
		t.Fatalf("logs = %v; want one warning %q", logs.lines, message)
	}
}

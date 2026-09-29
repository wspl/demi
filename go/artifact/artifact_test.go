package artifact_test

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/artifact/artifacttest"
)

var body = []byte("verified bytes")

func declared(data []byte) artifact.Digest {
	sum := sha256.Sum256(data)
	return artifact.Digest{Size: uint64(len(data)), SHA256: hex.EncodeToString(sum[:])}
}

// serve starts a fixture server that answers /artifact with status and data,
// declaring its length unless unsized.
func serve(t *testing.T, status int, data []byte, unsized bool) *artifacttest.Server {
	t.Helper()
	return artifacttest.Start(t, map[string]artifacttest.Answer{
		"/artifact": {Status: status, Body: data, Unsized: unsized},
	})
}

func TestADownloadIsVerifiedAsItArrives(t *testing.T) {
	ctx := t.Context()
	client := artifact.NewClientAllowingHTTP()
	server := serve(t, 200, body, false)
	url := server.URL("/artifact")
	var output bytes.Buffer
	if err := artifact.Download(ctx, client, url, declared(body), &output); err != nil || !bytes.Equal(output.Bytes(), body) {
		t.Fatalf("a verified download: %v, %q", err, output.Bytes())
	}
	wrong := declared(body)
	wrong.SHA256 = strings.Repeat("0", 64)
	if err := artifact.Download(ctx, client, url, wrong, &bytes.Buffer{}); !errors.Is(err, artifact.ErrDigest) {
		t.Errorf("a wrong digest: %v", err)
	}
	// A declared size that the server's length contradicts fails before the body.
	short := artifact.Digest{Size: 3, SHA256: declared(body).SHA256}
	var size *artifact.SizeError
	if err := artifact.Download(ctx, client, url, short, &bytes.Buffer{}); !errors.As(err, &size) || size.Declared != 3 || size.Actual != 14 {
		t.Errorf("a length the declared size contradicts: %v", err)
	}
	// Without a length, bytes past the declared size stop the download.
	unsized := serve(t, 200, body, true)
	var tooLarge *artifact.TooLargeError
	if err := artifact.Download(ctx, client, unsized.URL("/artifact"), short, &bytes.Buffer{}); !errors.As(err, &tooLarge) || tooLarge.Declared != 3 {
		t.Errorf("bytes past the declared size: %v", err)
	}
	// Without a length, a body shorter than declared is a size that differs.
	long := artifact.Digest{Size: 15, SHA256: declared(body).SHA256}
	if err := artifact.Download(ctx, client, unsized.URL("/artifact"), long, &bytes.Buffer{}); !errors.As(err, &size) || size.Declared != 15 || size.Actual != 14 {
		t.Errorf("a body shorter than declared: %v", err)
	}
	var rejected *artifact.RejectedError
	if err := artifact.Download(ctx, client, server.URL("/elsewhere"), declared(body), &bytes.Buffer{}); !errors.As(err, &rejected) || rejected.Status != 404 {
		t.Errorf("a missing artifact: %v", err)
	}
	// A redirect is an answer that is not the artifact, though it leads to it.
	moved := artifacttest.Start(t, map[string]artifacttest.Answer{
		"/moved":    {Status: 302, Location: "/artifact"},
		"/artifact": artifacttest.OK(body),
	})
	if err := artifact.Download(ctx, client, moved.URL("/moved"), declared(body), &bytes.Buffer{}); !errors.As(err, &rejected) || rejected.Status != 302 {
		t.Errorf("a redirect: %v", err)
	}
	if moved.Requests() != 1 {
		t.Errorf("the redirect was followed: %d requests", moved.Requests())
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if err := artifact.Download(cancelled, client, url, declared(body), &bytes.Buffer{}); !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled download: %v", err)
	}
}

func TestACancelledDownloadStopsWhileItWaitsForTheServer(t *testing.T) {
	hold := make(chan struct{})
	server := artifacttest.Start(t, map[string]artifacttest.Answer{"/artifact": {Status: 200, Body: body, Hold: hold}})
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() {
		done <- artifact.Download(ctx, artifact.NewClientAllowingHTTP(), server.URL("/artifact"), declared(body), &bytes.Buffer{})
	}()
	<-server.Arrived()
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Errorf("a download cancelled while it waits: %v", err)
	}
	close(hold)
}

func TestAMeasuredDownloadReportsWhatArrivedWithinItsLimit(t *testing.T) {
	ctx := t.Context()
	client := artifact.NewClientAllowingHTTP()
	server := serve(t, 200, body, false)
	var output bytes.Buffer
	measured, err := artifact.DownloadMeasured(ctx, client, server.URL("/artifact"), 1024, &output)
	if err != nil || measured != declared(body) || !bytes.Equal(output.Bytes(), body) {
		t.Fatalf("a measured download: %v, %+v", err, measured)
	}
	// A declared length past the limit fails before the body, and without one,
	// bytes past the limit stop the download.
	var tooLarge *artifact.TooLargeError
	if _, err := artifact.DownloadMeasured(ctx, client, server.URL("/artifact"), 3, &bytes.Buffer{}); !errors.As(err, &tooLarge) || tooLarge.Declared != 3 {
		t.Errorf("a length past the limit: %v", err)
	}
	unsized := serve(t, 200, body, true)
	if _, err := artifact.DownloadMeasured(ctx, client, unsized.URL("/artifact"), 3, &bytes.Buffer{}); !errors.As(err, &tooLarge) || tooLarge.Declared != 3 {
		t.Errorf("bytes past the limit: %v", err)
	}
	var rejected *artifact.RejectedError
	if _, err := artifact.DownloadMeasured(ctx, client, server.URL("/elsewhere"), 1024, &bytes.Buffer{}); !errors.As(err, &rejected) || rejected.Status != 404 {
		t.Errorf("a missing artifact: %v", err)
	}
}

func TestTheDownloadClientRefusesPlainHTTP(t *testing.T) {
	server := serve(t, 200, body, false)
	err := artifact.Download(t.Context(), artifact.NewClient(), server.URL("/artifact"), declared(body), &bytes.Buffer{})
	var failure *artifact.DownloadError
	if !errors.As(err, &failure) {
		t.Fatalf("a plain HTTP download: %v", err)
	}
	if server.Requests() != 0 {
		t.Errorf("the client asked the server %d times", server.Requests())
	}
}

func TestAFailedRequestNamesNoURL(t *testing.T) {
	const secret = "s3cr3t-signature"
	// Nothing listens at the address of a server that has stopped.
	server := artifacttest.Start(t, nil)
	url := server.URL("/artifact?signature=" + secret)
	server.Stop()
	err := artifact.Download(t.Context(), artifact.NewClientAllowingHTTP(), url, declared(body), &bytes.Buffer{})
	var failure *artifact.DownloadError
	if !errors.As(err, &failure) {
		t.Fatalf("a failed request: %v", err)
	}
	if strings.Contains(err.Error(), secret) {
		t.Errorf("the message quotes the URL: %v", err)
	}
}

func TestCopiesAndDigestsCheckTheDeclaredBytes(t *testing.T) {
	ctx := t.Context()
	source := filepath.Join(t.TempDir(), "source")
	if err := os.WriteFile(source, body, 0o644); err != nil {
		t.Fatal(err)
	}
	if got, err := artifact.DigestFile(ctx, source, 1024); err != nil || got != declared(body) {
		t.Errorf("digest = %+v, %v", got, err)
	}
	var tooLarge *artifact.TooLargeError
	if _, err := artifact.DigestFile(ctx, source, 3); !errors.As(err, &tooLarge) || tooLarge.Declared != 3 {
		t.Errorf("a file over the limit: %v", err)
	}
	var output bytes.Buffer
	if err := artifact.Copy(ctx, bytes.NewReader(body), declared(body), &output); err != nil || !bytes.Equal(output.Bytes(), body) {
		t.Errorf("a copy: %v", err)
	}
	longer := artifact.Digest{Size: uint64(len(body)) + 1, SHA256: declared(body).SHA256}
	var size *artifact.SizeError
	if err := artifact.Copy(ctx, bytes.NewReader(body), longer, &bytes.Buffer{}); !errors.As(err, &size) {
		t.Errorf("a copy shorter than declared: %v", err)
	}
	verifier := artifact.NewVerifier(declared([]byte("ab")))
	if _, err := verifier.Write([]byte("a")); err != nil {
		t.Error(err)
	}
	if _, err := verifier.Write([]byte("b")); err != nil {
		t.Error(err)
	}
	if _, err := verifier.Write([]byte("c")); !errors.As(err, &tooLarge) {
		t.Errorf("a byte past the declared size: %v", err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if _, err := artifact.DigestFile(cancelled, source, 1024); !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled digest: %v", err)
	}
}

func publication(mode artifact.Mode, permissions artifact.Permissions) artifact.Publication {
	return artifact.Publication{Mode: mode, Permissions: permissions, Durable: true}
}

// names returns the names in directory, sorted.
func names(t *testing.T, directory string) []string {
	t.Helper()
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	slices.Sort(names)
	return names
}

func TestPublicationCreatesOrReplacesWholeFiles(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "file")
	if err := artifact.PublishBytes(path, []byte("first"), publication(artifact.CreateNew, artifact.DefaultPermissions)); err != nil {
		t.Fatal(err)
	}
	if err := artifact.PublishBytes(path, []byte("second"), publication(artifact.CreateNew, artifact.DefaultPermissions)); !errors.Is(err, fs.ErrExist) {
		t.Errorf("a second creation: %v", err)
	}
	if got, _ := os.ReadFile(path); string(got) != "first" {
		t.Errorf("the file holds %q after a refused creation", got)
	}
	if err := artifact.Publish(t.Context(), path, bytes.NewReader([]byte("replaced")), publication(artifact.Replace, artifact.DefaultPermissions)); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(path); string(got) != "replaced" {
		t.Errorf("the file holds %q", got)
	}
	// Nothing but the file remains beside it, after the refusal too.
	if got := names(t, directory); !slices.Equal(got, []string{"file"}) {
		t.Errorf("the directory holds %v", got)
	}
}

func TestACancelledPublicationPublishesNothing(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "output")
	cancelled, cancel := context.WithCancel(t.Context())
	cancel()
	// The whole input is ready at once, so only the cancellation stops it.
	err := artifact.Publish(cancelled, path, bytes.NewReader(body), publication(artifact.CreateNew, artifact.DefaultPermissions))
	if !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled publication: %v", err)
	}
	// Neither the file nor its staged copy is left.
	if got := names(t, directory); len(got) != 0 {
		t.Errorf("the directory holds %v", got)
	}
	// A cancelled replacement leaves the old file.
	if err := os.WriteFile(path, []byte("old"), 0o644); err != nil {
		t.Fatal(err)
	}
	err = artifact.Publish(cancelled, path, bytes.NewReader([]byte("never")), publication(artifact.Replace, artifact.DefaultPermissions))
	if got, _ := os.ReadFile(path); !errors.Is(err, context.Canceled) || string(got) != "old" {
		t.Errorf("a cancelled replacement: %v, the file holds %q", err, got)
	}
}

func TestAStagedFileAppearsOnlyWhenPublished(t *testing.T) {
	const prefix = ".demi-partial-"
	directory := t.TempDir()
	path := filepath.Join(directory, "tool")
	abandoned, err := artifact.Stage(path, publication(artifact.CreateNew, artifact.Executable))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := abandoned.File().WriteString("partial"); err != nil {
		t.Fatal(err)
	}
	// Its name says whose it is, should its process die before it is published
	// (docs/execution/runner.md § File contents).
	if got := names(t, directory); len(got) != 1 || !strings.HasPrefix(got[0], prefix) {
		t.Errorf("a staged file is named %v", got)
	}
	abandoned.Discard()
	if got := names(t, directory); len(got) != 0 {
		t.Errorf("a discarded file left %v", got)
	}
	staged, err := artifact.Stage(path, publication(artifact.CreateNew, artifact.Executable))
	if err != nil {
		t.Fatal(err)
	}
	defer staged.Discard()
	if err := artifact.Copy(t.Context(), bytes.NewReader(body), declared(body), staged.File()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("a staged file is at its path: %v", err)
	}
	if err := staged.Publish(); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(path); !bytes.Equal(got, body) {
		t.Errorf("the published file holds %q", got)
	}
	if got := names(t, directory); !slices.Equal(got, []string{"tool"}) {
		t.Errorf("the directory holds %v", got)
	}
}

func TestPublicationSetsOrKeepsPermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows has no Unix permissions")
	}
	directory := t.TempDir()
	mode := func(path string) fs.FileMode {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		return info.Mode().Perm()
	}
	private := filepath.Join(directory, "private")
	if err := artifact.PublishBytes(private, []byte("secret"), publication(artifact.CreateNew, artifact.Private)); err != nil {
		t.Fatal(err)
	}
	if got := mode(private); got != 0o600 {
		t.Errorf("a private file has mode %o", got)
	}
	tool := filepath.Join(directory, "tool")
	if err := artifact.PublishBytes(tool, []byte("#!/bin/sh\n"), publication(artifact.CreateNew, artifact.Executable)); err != nil {
		t.Fatal(err)
	}
	if got := mode(tool); got != 0o755 {
		t.Errorf("an executable has mode %o", got)
	}
	if err := os.Chmod(private, 0o640); err != nil {
		t.Fatal(err)
	}
	if err := artifact.PublishBytes(private, []byte("rotated"), publication(artifact.Replace, artifact.Keep)); err != nil {
		t.Fatal(err)
	}
	if got := mode(private); got != 0o640 {
		t.Errorf("a kept file has mode %o", got)
	}
}

func TestAStagedDirectoryReplacesTheInstalledOne(t *testing.T) {
	root := t.TempDir()
	installed := filepath.Join(root, "1.0.0")
	staged := filepath.Join(root, "staged")
	for path, name := range map[string]string{installed: "old", staged: "new"} {
		if err := os.Mkdir(path, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(path, name), []byte(name), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if err := artifact.PublishDirectory(staged, installed); err != nil {
		t.Fatal(err)
	}
	if got := names(t, root); !slices.Equal(got, []string{"1.0.0"}) {
		t.Errorf("the root holds %v", got)
	}
	if got := names(t, installed); !slices.Equal(got, []string{"new"}) {
		t.Errorf("the installation holds %v", got)
	}
}

func TestAReleaseIsPublishedWholeOnceAndRefusedOverOtherContents(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	sources := filepath.Join(root, "sources")
	if err := os.Mkdir(sources, 0o755); err != nil {
		t.Fatal(err)
	}
	for name, data := range map[string]string{"tool": "tool v1", "data": "data"} {
		if err := os.WriteFile(filepath.Join(sources, name), []byte(data), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	files := []artifact.ReleaseFile{
		{Source: filepath.Join(sources, "tool"), Path: "x86_64-unknown-linux-musl/tool", Digest: declared([]byte("tool v1")), Executable: true},
		{Source: filepath.Join(sources, "data"), Path: "data", Digest: declared([]byte("data"))},
	}
	record := artifact.ReleaseRecord{Name: "descriptor.json", Bytes: []byte("{\"version\":\"1\"}\n")}
	releases := filepath.Join(root, "releases")
	directory := filepath.Join(releases, "tool-1")
	if err := artifact.PublishRelease(ctx, directory, record, files); err != nil {
		t.Fatal(err)
	}
	read := func(path string) string {
		data, _ := os.ReadFile(filepath.Join(directory, path))
		return string(data)
	}
	if read("descriptor.json") != string(record.Bytes) || read("x86_64-unknown-linux-musl/tool") != "tool v1" || read("data") != "data" {
		t.Errorf("the release holds %v", names(t, directory))
	}
	if runtime.GOOS != "windows" {
		for path, want := range map[string]fs.FileMode{"x86_64-unknown-linux-musl/tool": 0o111, "data": 0} {
			info, err := os.Stat(filepath.Join(directory, path))
			if err != nil || info.Mode()&0o111 != want {
				t.Errorf("%s has mode %v, %v", path, info.Mode(), err)
			}
		}
	}
	// The same release again is the one in place.
	if err := artifact.PublishRelease(ctx, directory, record, files); err != nil {
		t.Errorf("the same release again: %v", err)
	}
	// Another record under the same name is refused, and so is a file that
	// changed in place; nothing in place changes.
	other := artifact.ReleaseRecord{Name: "descriptor.json", Bytes: []byte("{\"version\":\"2\"}\n")}
	var conflict *artifact.ConflictError
	if err := artifact.PublishRelease(ctx, directory, other, files); !errors.As(err, &conflict) || filepath.Base(conflict.Path) != "descriptor.json" {
		t.Errorf("another record: %v", err)
	}
	if err := os.WriteFile(filepath.Join(directory, "data"), []byte("corrupt"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := artifact.PublishRelease(ctx, directory, record, files); !errors.As(err, &conflict) || filepath.Base(conflict.Path) != "data" {
		t.Errorf("a file changed in place: %v", err)
	}
	if read("descriptor.json") != string(record.Bytes) {
		t.Error("the record in place changed")
	}
	// A source that is not what its release declares publishes nothing.
	if err := os.WriteFile(filepath.Join(sources, "tool"), []byte("tool v2"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := artifact.PublishRelease(ctx, filepath.Join(releases, "tool-2"), record, files); !errors.Is(err, artifact.ErrDigest) {
		t.Errorf("a source that changed: %v", err)
	}
	// No stage is left beside the releases, whatever happened.
	if got := names(t, releases); !slices.Equal(got, []string{"tool-1"}) {
		t.Errorf("the releases hold %v", got)
	}
}

func TestAReleaseThatBreaksItsShapeIsRefusedBeforeAnythingIsWritten(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "data")
	if err := os.WriteFile(source, []byte("data"), 0o644); err != nil {
		t.Fatal(err)
	}
	file := artifact.ReleaseFile{Source: source, Path: "data", Digest: declared([]byte("data"))}
	escaping := file
	escaping.Path = "../escaped"
	current, parent, inside := file, file, file
	current.Path = "."
	parent.Path = "a/.."
	inside.Path = "a/./b"
	for name, test := range map[string]struct {
		directory string
		record    artifact.ReleaseRecord
		files     []artifact.ReleaseFile
	}{
		"a record that is a path":         {filepath.Join(root, "releases", "one"), artifact.ReleaseRecord{Name: "a/b"}, nil},
		"a release with no parent":        {"release", artifact.ReleaseRecord{Name: "descriptor.json"}, nil},
		"a file outside its release":      {filepath.Join(root, "releases", "one"), artifact.ReleaseRecord{Name: "descriptor.json"}, []artifact.ReleaseFile{escaping}},
		"a file that is the release":      {filepath.Join(root, "releases", "one"), artifact.ReleaseRecord{Name: "descriptor.json"}, []artifact.ReleaseFile{current}},
		"a file below its own parent":     {filepath.Join(root, "releases", "one"), artifact.ReleaseRecord{Name: "descriptor.json"}, []artifact.ReleaseFile{parent}},
		"a file with a current directory": {filepath.Join(root, "releases", "one"), artifact.ReleaseRecord{Name: "descriptor.json"}, []artifact.ReleaseFile{inside}},
	} {
		if err := artifact.PublishRelease(t.Context(), test.directory, test.record, test.files); !errors.Is(err, fs.ErrInvalid) {
			t.Errorf("%s: %v", name, err)
		}
	}
	if _, err := os.Stat(filepath.Join(root, "releases")); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("a refused release made its parent directory: %v", err)
	}
}

// The clock is fake: a waiter tries the lock again every 50 ms of it, so each
// pause below lets it find the lock held a few times, at once.
func TestOneInstallerHoldsTheLockAndAWaiterCanGiveUp(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "artifact.lock")
		var waits artifacttest.LockWaits
		ctx := waits.Watch(t.Context())
		held, err := artifact.AcquireInstallLock(ctx, path)
		if err != nil {
			t.Fatal(err)
		}
		type outcome struct {
			lock *artifact.InstallLock
			err  error
		}
		acquire := func(ctx context.Context) <-chan outcome {
			result := make(chan outcome, 1)
			go func() {
				lock, err := artifact.AcquireInstallLock(ctx, path)
				result <- outcome{lock, err}
			}()
			return result
		}
		start := time.Now()
		waiting := acquire(ctx)
		synctest.Wait()
		select {
		case <-waiting:
			t.Fatal("a second installer took a lock that is held")
		default:
		}
		if waits.Count() != 1 {
			t.Errorf("%d waits, want 1 (a waiter counts once, however often it tries)", waits.Count())
		}
		// The waiter tries again every 50 ms: the lock let go after 120 ms is taken
		// at 150 ms.
		time.Sleep(120 * time.Millisecond)
		if err := held.Close(); err != nil {
			t.Fatal(err)
		}
		next := <-waiting
		if next.err != nil {
			t.Fatalf("the lock does not pass on: %v", next.err)
		}
		if elapsed := time.Since(start); elapsed != 150*time.Millisecond {
			t.Errorf("the lock passed on after %v, want 150 ms", elapsed)
		}
		giving, giveUp := context.WithCancel(ctx)
		quitter := acquire(giving)
		synctest.Wait()
		giveUp()
		if quit := <-quitter; !errors.Is(quit.err, context.Canceled) {
			t.Errorf("a waiter told to give up: %v", quit.err)
		}
		next.lock.Close()
	})
}

func newArchive(t *testing.T, entries map[string][]byte) (artifact.Archive, *artifacttest.Server) {
	t.Helper()
	data := artifacttest.Zip(t, entries)
	server := artifacttest.Start(t, map[string]artifacttest.Answer{"/app.zip": artifacttest.OK(data)})
	return artifact.Archive{URL: server.URL("/app.zip"), Digest: declared(data), Executable: "app/bin/tool"}, server
}

func TestAnArchiveIsInstalledOnceAndCheckedBeforeEachUse(t *testing.T) {
	ctx := t.Context()
	client := artifact.NewClientAllowingHTTP()
	archive, server := newArchive(t, map[string][]byte{"app/bin/tool": []byte("tool"), "app/data": []byte("data")})
	root := t.TempDir()
	directory := filepath.Join(root, archive.Digest.SHA256)
	if _, ok, err := artifact.Installed(ctx, directory, archive); ok || err != nil {
		t.Fatalf("before installing: %v, %v", ok, err)
	}
	// Two installers at once: one downloads, the other finds its result.
	var wait sync.WaitGroup
	executables := make([]string, 2)
	errs := make([]error, 2)
	for i := range executables {
		wait.Add(1)
		go func() {
			defer wait.Done()
			executables[i], errs[i] = artifact.InstallArchive(ctx, client, root, archive)
		}()
	}
	wait.Wait()
	executable := filepath.Join(directory, "app", "bin", "tool")
	if errs[0] != nil || errs[1] != nil || executables[0] != executable || executables[1] != executable {
		t.Fatalf("two installers: %v %v, %q %q", errs[0], errs[1], executables[0], executables[1])
	}
	if got, _ := os.ReadFile(executable); string(got) != "tool" {
		t.Errorf("the executable holds %q", got)
	}
	if got, _ := os.ReadFile(filepath.Join(directory, "app", "data")); string(got) != "data" {
		t.Errorf("the data holds %q", got)
	}
	if server.Requests() != 1 {
		t.Errorf("the archive was downloaded %d times", server.Requests())
	}
	if found, ok, err := artifact.Installed(ctx, directory, archive); !ok || err != nil || found != executable {
		t.Errorf("installed = %q, %v, %v", found, ok, err)
	}
	// Nothing but the installation and its lock stays in the root.
	lock := archive.Digest.SHA256 + ".lock"
	if got := names(t, root); !slices.Equal(got, []string{archive.Digest.SHA256, lock}) {
		t.Errorf("the root holds %v", got)
	}
	// A changed executable fails the check, and a new install neither replaces it
	// nor downloads again.
	if err := os.WriteFile(executable, []byte("changed"), 0o755); err != nil {
		t.Fatal(err)
	}
	var installation *artifact.InstallationError
	if _, _, err := artifact.Installed(ctx, directory, archive); !errors.As(err, &installation) {
		t.Errorf("a changed executable: %v", err)
	}
	if _, err := artifact.InstallArchive(ctx, client, root, archive); !errors.As(err, &installation) {
		t.Errorf("installing over a changed executable: %v", err)
	}
	if server.Requests() != 1 {
		t.Errorf("a changed installation was downloaded again")
	}
	if err := os.Remove(filepath.Join(directory, artifact.ReceiptFile)); err != nil {
		t.Fatal(err)
	}
	if _, _, err := artifact.Installed(ctx, directory, archive); !errors.As(err, &installation) {
		t.Errorf("an installation without a receipt: %v", err)
	}
	// An archive that is not the declared one, or lacks its executable, installs
	// nothing.
	other := t.TempDir()
	wrong := archive
	wrong.Digest.SHA256 = strings.Repeat("0", 64)
	if _, err := artifact.InstallArchive(ctx, client, other, wrong); !errors.Is(err, artifact.ErrDigest) {
		t.Errorf("an archive that is not the declared one: %v", err)
	}
	lacking := archive
	lacking.Executable = "app/bin/other"
	var archiveError *artifact.ArchiveError
	if _, err := artifact.InstallArchive(ctx, client, other, lacking); !errors.As(err, &archiveError) {
		t.Errorf("an archive without its executable: %v", err)
	}
	wantLocks := []string{wrong.Digest.SHA256 + ".lock", lock}
	slices.Sort(wantLocks)
	if got := names(t, other); !slices.Equal(got, wantLocks) {
		t.Errorf("the failed installs left %v", got)
	}
}

func TestAnArchiveExtractsInsideItsInstallationOnlyAndNamesItsFiles(t *testing.T) {
	ctx := t.Context()
	client := artifact.NewClientAllowingHTTP()
	root := t.TempDir()
	installs := filepath.Join(root, "installs")
	for name, entries := range map[string]map[string][]byte{
		"an entry that leaves the installation": {"app/bin/tool": []byte("tool"), "../escaped": []byte("no")},
		"an absolute entry":                     {"app/bin/tool": []byte("tool"), "/escaped": []byte("no")},
	} {
		archive, _ := newArchive(t, entries)
		var archiveError *artifact.ArchiveError
		if _, err := artifact.InstallArchive(ctx, client, installs, archive); !errors.As(err, &archiveError) {
			t.Errorf("%s: %v", name, err)
		}
		if _, err := os.Stat(filepath.Join(root, "escaped")); !errors.Is(err, fs.ErrNotExist) {
			t.Errorf("%s: a file escaped: %v", name, err)
		}
	}
	file := filepath.Join(root, "app.zip")
	if err := os.WriteFile(file, artifacttest.Zip(t, map[string][]byte{"app/bin/tool": []byte("tool")}), 0o644); err != nil {
		t.Fatal(err)
	}
	for path, want := range map[string]bool{"app/bin/tool": true, "app/bin": false, "app/bin/other": false} {
		if got, err := artifact.ZipHolds(file, path); err != nil || got != want {
			t.Errorf("ZipHolds(%q) = %v, %v; want %v", path, got, err, want)
		}
	}
	if err := os.WriteFile(file, []byte("not a zip archive"), 0o644); err != nil {
		t.Fatal(err)
	}
	var archiveError *artifact.ArchiveError
	if _, err := artifact.ZipHolds(file, "app/bin/tool"); !errors.As(err, &archiveError) {
		t.Errorf("a file that is not an archive: %v", err)
	}
}

func TestAnArchiveMakesSymbolicLinksButWritesNothingThroughThem(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows needs a privilege to make symbolic links")
	}
	outside := t.TempDir()
	build := func(entries ...string) []byte {
		var archive bytes.Buffer
		writer := zip.NewWriter(&archive)
		for _, entry := range entries {
			header := &zip.FileHeader{Name: entry, Method: zip.Store}
			contents := "tool"
			if entry == "app/link" {
				header.SetMode(fs.ModeSymlink | 0o777)
				contents = outside
			}
			file, err := writer.CreateHeader(header)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := file.Write([]byte(contents)); err != nil {
				t.Fatal(err)
			}
		}
		if err := writer.Close(); err != nil {
			t.Fatal(err)
		}
		return archive.Bytes()
	}
	install := func(data []byte) (string, error) {
		server := artifacttest.Start(t, map[string]artifacttest.Answer{"/app.zip": artifacttest.OK(data)})
		archive := artifact.Archive{URL: server.URL("/app.zip"), Digest: declared(data), Executable: "app/bin/tool"}
		return artifact.InstallArchive(t.Context(), artifact.NewClientAllowingHTTP(), t.TempDir(), archive)
	}
	// A link whose target is elsewhere is made, as the archive says.
	executable, err := install(build("app/bin/tool", "app/link"))
	if err != nil {
		t.Fatal(err)
	}
	if target, err := os.Readlink(filepath.Join(filepath.Dir(filepath.Dir(executable)), "link")); err != nil || target != outside {
		t.Errorf("the link points to %q, %v", target, err)
	}
	// A file written through it is refused: it would leave the installation.
	var archiveError *artifact.ArchiveError
	if _, err := install(build("app/link", "app/link/pwned", "app/bin/tool")); !errors.As(err, &archiveError) {
		t.Errorf("a write through a link: %v", err)
	}
	if got := names(t, outside); len(got) != 0 {
		t.Errorf("a file was written outside: %v", got)
	}
}

func TestAnArchiveNeverWritesThroughASymbolicLink(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows needs a privilege to make symbolic links")
	}
	build := func(entries ...entry) []byte {
		var archive bytes.Buffer
		writer := zip.NewWriter(&archive)
		for _, e := range entries {
			header := &zip.FileHeader{Name: e.name, Method: zip.Store}
			if e.link {
				header.SetMode(fs.ModeSymlink | 0o777)
			}
			file, err := writer.CreateHeader(header)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := file.Write([]byte(e.contents)); err != nil {
				t.Fatal(err)
			}
		}
		if err := writer.Close(); err != nil {
			t.Fatal(err)
		}
		return archive.Bytes()
	}
	install := func(data []byte) (string, error) {
		server := artifacttest.Start(t, map[string]artifacttest.Answer{"/app.zip": artifacttest.OK(data)})
		archive := artifact.Archive{URL: server.URL("/app.zip"), Digest: declared(data), Executable: "app/bin/tool"}
		return artifact.InstallArchive(t.Context(), artifact.NewClientAllowingHTTP(), t.TempDir(), archive)
	}
	var archiveError *artifact.ArchiveError
	for name, entries := range map[string][]entry{
		"a file below a link to a directory of the archive": {{name: "app/bin/tool", contents: "tool"}, {name: "app/real/", contents: ""}, {name: "app/link", contents: "real", link: true}, {name: "app/link/pwned", contents: "no"}},
		"a file at a link":                   {{name: "app/bin/tool", contents: "tool"}, {name: "app/real", contents: "x"}, {name: "app/link", contents: "real", link: true}, {name: "app/link", contents: "no"}},
		"a link below a link to a directory": {{name: "app/bin/tool", contents: "tool"}, {name: "app/real/", contents: ""}, {name: "app/link", contents: "real", link: true}, {name: "app/link/inner", contents: "bin/tool", link: true}},
		"a directory below a link":           {{name: "app/bin/tool", contents: "tool"}, {name: "app/real/", contents: ""}, {name: "app/link", contents: "real", link: true}, {name: "app/link/dir/", contents: ""}},
	} {
		if _, err := install(build(entries...)); !errors.As(err, &archiveError) {
			t.Errorf("%s: %v", name, err)
		}
	}
	// A link on its own is made.
	if _, err := install(build(entry{name: "app/bin/tool", contents: "tool"}, entry{name: "app/link", contents: "bin/tool", link: true})); err != nil {
		t.Errorf("a link on its own: %v", err)
	}
}

type entry struct {
	name, contents string
	link           bool
}

func TestAnEntryWithoutAttributesKeepsTheDefaultPermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows has no Unix permissions")
	}
	const unixCreator = 3 << 8
	var archive bytes.Buffer
	writer := zip.NewWriter(&archive)
	for name, header := range map[string]*zip.FileHeader{
		"app/bin/tool": {Name: "app/bin/tool", CreatorVersion: unixCreator, ExternalAttrs: 0o100755 << 16, Method: zip.Store},
		"app/plain":    {Name: "app/plain", CreatorVersion: unixCreator, Method: zip.Store},
		"app/private":  {Name: "app/private", CreatorVersion: unixCreator, ExternalAttrs: 0o100600 << 16, Method: zip.Store},
	} {
		file, err := writer.CreateHeader(header)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := file.Write([]byte(name)); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	data := archive.Bytes()
	server := artifacttest.Start(t, map[string]artifacttest.Answer{"/app.zip": artifacttest.OK(data)})
	archiveToInstall := artifact.Archive{URL: server.URL("/app.zip"), Digest: declared(data), Executable: "app/bin/tool"}
	executable, err := artifact.InstallArchive(t.Context(), artifact.NewClientAllowingHTTP(), t.TempDir(), archiveToInstall)
	if err != nil {
		t.Fatal(err)
	}
	app := filepath.Dir(filepath.Dir(executable))
	for path, want := range map[string]fs.FileMode{"bin/tool": 0o755, "private": 0o600} {
		if info, err := os.Stat(filepath.Join(app, path)); err != nil || info.Mode().Perm() != want {
			t.Errorf("%s has %v, %v; want %o", path, info.Mode(), err, want)
		}
	}
	// The default is the usual permissions of a new file, less the umask: never
	// none.
	if info, err := os.Stat(filepath.Join(app, "plain")); err != nil || info.Mode().Perm()&0o600 != 0o600 {
		t.Errorf("an entry without attributes has %v, %v", info.Mode(), err)
	}
}

func TestZipHoldsASymbolicLinkNoMoreThanADirectoryAndPassesTheOSErrorsOn(t *testing.T) {
	root := t.TempDir()
	file := filepath.Join(root, "app.zip")
	var archive bytes.Buffer
	writer := zip.NewWriter(&archive)
	header := &zip.FileHeader{Name: "app/link", Method: zip.Store}
	header.SetMode(fs.ModeSymlink | 0o777)
	link, err := writer.CreateHeader(header)
	if err != nil {
		t.Fatal(err)
	}
	link.Write([]byte("tool"))
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(file, archive.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
	if got, err := artifact.ZipHolds(file, "app/link"); err != nil || got {
		t.Errorf("a symbolic link: %v, %v", got, err)
	}
	var archiveError *artifact.ArchiveError
	if _, err := artifact.ZipHolds(filepath.Join(root, "missing.zip"), "x"); !errors.Is(err, fs.ErrNotExist) || errors.As(err, &archiveError) {
		t.Errorf("a missing archive: %v", err)
	}
	if runtime.GOOS != "windows" && os.Getuid() != 0 {
		if err := os.Chmod(file, 0); err != nil {
			t.Fatal(err)
		}
		if _, err := artifact.ZipHolds(file, "x"); !errors.Is(err, fs.ErrPermission) || errors.As(err, &archiveError) {
			t.Errorf("an unreadable archive: %v", err)
		}
	}
}

func TestADownloadAsksForTheBytesAsTheyAreNotCompressed(t *testing.T) {
	server := serve(t, 200, body, false)
	if err := artifact.Download(t.Context(), artifact.NewClientAllowingHTTP(), server.URL("/artifact"), declared(body), &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	// A body that the transport decompressed would not be the bytes whose size
	// and digest are declared.
	if head := strings.ToLower(server.LastRequest()); strings.Contains(head, "accept-encoding") {
		t.Errorf("the request asks for an encoding: %q", head)
	}
}

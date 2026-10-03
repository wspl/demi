package artifacts_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

var body = []byte("verified bytes")

func declared(data []byte) artifacts.Digest {
	hash := sha256.Sum256(data)
	return artifacts.Digest{Size: uint64(len(data)), SHA256: hex.EncodeToString(hash[:])}
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func contents(t *testing.T, path string, expected []byte) {
	t.Helper()
	found, err := os.ReadFile(path)
	must(t, err)
	if !bytes.Equal(found, expected) {
		t.Fatalf("%s: got %q, want %q", path, found, expected)
	}
}

func names(t *testing.T, directory string) []string {
	t.Helper()
	entries, err := os.ReadDir(directory)
	must(t, err)
	found := make([]string, 0, len(entries))
	for _, entry := range entries {
		found = append(found, entry.Name())
	}
	return found
}

func client(t *testing.T) *artifacts.Client {
	t.Helper()
	c := artifacts.NewClientAllowingHTTP()
	t.Cleanup(c.Close)
	return c
}

// assertPrefix checks that err's text starts with prefix.
func assertPrefix(t *testing.T, err error, prefix string) {
	t.Helper()
	if err == nil || !strings.HasPrefix(err.Error(), prefix) {
		t.Fatalf("got %v, want an error starting %q", err, prefix)
	}
}

func assertError[T error](t *testing.T, err error) T {
	t.Helper()
	var expected T
	if !errors.As(err, &expected) {
		t.Fatalf("got %v, want %T", err, expected)
	}
	return expected
}

// Each scenario uses local fixtures and finishes in well under one second.
func TestDownloadVerifiesAsBytesArrive(t *testing.T) {
	fixtures := map[string]artifactstest.Answer{
		"/artifact": artifactstest.OK(body),
		"/unsized":  {Status: 200, Body: body},
		"/moved":    {Status: 302, Length: true},
	}
	server := artifactstest.Start(t, fixtures)
	c := client(t)
	var output bytes.Buffer
	must(t, artifacts.Download(t.Context(), c, server.URL("/artifact"), declared(body), &output))
	if !bytes.Equal(output.Bytes(), body) {
		t.Fatal("wrong downloaded bytes")
	}
	wrong := declared(body)
	wrong.SHA256 = strings.Repeat("0", 64)
	if err := artifacts.Download(
		t.Context(),
		c,
		server.URL("/artifact"),
		wrong,
		io.Discard,
	); !errors.Is(
		err,
		artifacts.ErrDigest,
	) {
		t.Fatalf("wrong hash: %v", err)
	}
	short := declared(body)
	short.Size = 3
	output.Reset()
	size := assertError[*artifacts.SizeError](
		t,
		artifacts.Download(t.Context(), c, server.URL("/artifact"), short, &output),
	)
	if size.Declared != 3 || output.Len() != 0 {
		t.Fatal("contradictory length was not rejected before writing")
	}
	tooLarge := assertError[*artifacts.TooLargeError](
		t,
		artifacts.Download(t.Context(), c, server.URL("/unsized"), short, io.Discard),
	)
	if tooLarge.Declared != 3 {
		t.Fatal(tooLarge)
	}
	for path, status := range map[string]int{"/missing": 404, "/moved": 302} {
		rejected := assertError[*artifacts.RejectedError](
			t,
			artifacts.Download(t.Context(), c, server.URL(path), declared(body), io.Discard),
		)
		if rejected.Status != status {
			t.Fatal(rejected)
		}
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := artifacts.Download(
		ctx,
		c,
		server.URL("/artifact"),
		declared(body),
		io.Discard,
	); !errors.Is(
		err,
		context.Canceled,
	) {
		t.Fatal(err)
	}
}

func TestDownloadDecodesZstdAndRefusesOtherCoding(t *testing.T) {
	for _, effort := range []artifacts.Effort{artifacts.Published, artifacts.Development} {
		encoded, err := artifacts.Encode(t.Context(), body, effort)
		must(t, err)
		if bytes.Equal(encoded, body) {
			t.Fatal("zstd encoding left bytes unchanged")
		}
		server := artifactstest.Start(t, map[string]artifactstest.Answer{
			"/zstd": {Status: 200, Body: encoded, Length: true, Coding: artifacts.ContentCoding},
			"/gzip": {Status: 200, Body: body, Length: true, Coding: "gzip"},
		})
		c := client(t)
		var output bytes.Buffer
		must(t, artifacts.Download(t.Context(), c, server.URL("/zstd"), declared(body), &output))
		if !bytes.Equal(output.Bytes(), body) {
			t.Fatal("did not decode executable")
		}
		found, err := artifacts.DownloadMeasured(t.Context(), c, server.URL("/zstd"), 1024, io.Discard)
		must(t, err)
		if found != declared(body) {
			t.Fatal(found)
		}
		coding := assertError[*artifacts.CodingError](
			t,
			artifacts.Download(t.Context(), c, server.URL("/gzip"), declared(body), io.Discard),
		)
		if coding.Coding != "gzip" {
			t.Fatal(coding)
		}
	}
}

func TestMeasuredDownloadReportsBytesWithinLimit(t *testing.T) {
	server := artifactstest.Start(t, map[string]artifactstest.Answer{
		"/sized":   artifactstest.OK(body),
		"/unsized": {Status: 200, Body: body},
	})
	c := client(t)
	for _, path := range []string{"/sized", "/unsized"} {
		var output bytes.Buffer
		found, err := artifacts.DownloadMeasured(t.Context(), c, server.URL(path), 1024, &output)
		must(t, err)
		if found != declared(body) || !bytes.Equal(output.Bytes(), body) {
			t.Fatal("incorrect measurement")
		}
		_, err = artifacts.DownloadMeasured(t.Context(), c, server.URL(path), 3, io.Discard)
		if tooLarge := assertError[*artifacts.TooLargeError](t, err); tooLarge.Declared != 3 {
			t.Fatal(tooLarge)
		}
	}
	_, err := artifacts.DownloadMeasured(t.Context(), c, server.URL("/missing"), 1024, io.Discard)
	if rejected := assertError[*artifacts.RejectedError](t, err); rejected.Status != 404 {
		t.Fatal(rejected)
	}
}

func TestClientRefusesHTTPAndHidesSignedURL(t *testing.T) {
	server := artifactstest.Start(t, map[string]artifactstest.Answer{"/artifact": artifactstest.OK(body)})
	c := artifacts.NewClient()
	defer c.Close()
	err := artifacts.Download(t.Context(), c, server.URL("/artifact?signature=secret"), declared(body), io.Discard)
	if err == nil || strings.Contains(err.Error(), "secret") || server.Requests() != 0 {
		t.Fatalf("HTTP request leaked: %v", err)
	}
}

func TestCancellationInterruptsWaitingDownload(t *testing.T) {
	requested := make(chan struct{})
	server := artifactstest.Start(
		t,
		map[string]artifactstest.Answer{
			"/artifact": {Status: 200, Body: body, Length: true, Requested: requested, Gate: make(chan struct{})},
		},
	)
	c := client(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	result := make(chan error, 1)
	go func() { result <- artifacts.Download(ctx, c, server.URL("/artifact"), declared(body), io.Discard) }()
	<-requested
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestCopiesAndDigestsCheckDeclaredBytes(t *testing.T) {
	source := filepath.Join(t.TempDir(), "source")
	must(t, os.WriteFile(source, body, 0o600))
	found, err := artifacts.DigestFile(t.Context(), source, 1024)
	must(t, err)
	if found != declared(body) {
		t.Fatal(found)
	}
	_, err = artifacts.DigestFile(t.Context(), source, 3)
	if tooLarge := assertError[*artifacts.TooLargeError](t, err); tooLarge.Declared != 3 {
		t.Fatal(tooLarge)
	}
	var output bytes.Buffer
	input, err := os.Open(source)
	must(t, err)
	defer func() { must(t, input.Close()) }()
	must(t, artifacts.Copy(t.Context(), input, declared(body), &output))
	if !bytes.Equal(output.Bytes(), body) {
		t.Fatal("copy changed bytes")
	}
	longer := declared(body)
	longer.Size++
	_ = assertError[*artifacts.SizeError](t, artifacts.Copy(t.Context(), bytes.NewReader(body), longer, io.Discard))
	verifier := artifacts.NewVerifier(declared([]byte("ab")))
	must(t, verifier.Update([]byte("a")))
	must(t, verifier.Update([]byte("b")))
	must(t, verifier.Finish())
	_ = assertError[*artifacts.TooLargeError](t, verifier.Update([]byte("c")))
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	_, err = artifacts.DigestFile(ctx, source, 1024)
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestPublicationCreatesOrReplacesWholeFiles(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "file")
	publication := artifacts.Publication{Mode: artifacts.CreateNew, Durable: true}
	must(t, artifacts.PublishBytes(t.Context(), path, []byte("first"), publication))
	if err := artifacts.PublishBytes(t.Context(), path, []byte("second"), publication); !errors.Is(err, os.ErrExist) {
		t.Fatal(err)
	}
	contents(t, path, []byte("first"))
	publication.Mode = artifacts.Replace
	must(t, artifacts.Publish(t.Context(), path, strings.NewReader("replaced"), publication))
	contents(t, path, []byte("replaced"))
	if got := names(t, directory); len(got) != 1 || got[0] != "file" {
		t.Fatal(got)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := artifacts.Publish(ctx, path, strings.NewReader("never"), publication); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	contents(t, path, []byte("replaced"))
	if got := names(t, directory); len(got) != 1 || got[0] != "file" {
		t.Fatal(got)
	}
}

func TestCancelledPublicationPublishesNothing(t *testing.T) {
	directory := t.TempDir()
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	err := artifacts.Publish(ctx, filepath.Join(directory, "output"), bytes.NewReader(body), artifacts.Publication{})
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	if got := names(t, directory); len(got) != 0 {
		t.Fatal(got)
	}
}

func TestStagedFileAppearsOnlyWhenPublished(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "tool")
	publication := artifacts.Publication{Permissions: artifacts.Executable, Durable: true}
	abandoned, err := artifacts.NewStaged(t.Context(), path, publication)
	must(t, err)
	defer func() { must(t, abandoned.Close()) }()
	_, err = abandoned.File().Write([]byte("partial"))
	must(t, err)
	got := names(t, directory)
	if len(got) != 1 || !strings.HasPrefix(got[0], ".demi-partial-") {
		t.Fatal(got)
	}
	must(t, abandoned.Close())
	if len(names(t, directory)) != 0 {
		t.Fatal("abandoned staging file remained")
	}
	staged, err := artifacts.NewStaged(t.Context(), path, publication)
	must(t, err)
	defer func() { must(t, staged.Close()) }()
	must(t, artifacts.Copy(t.Context(), bytes.NewReader(body), declared(body), staged.File()))
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("staged artifact visible")
	}
	must(t, staged.Publish(t.Context()))
	contents(t, path, body)
	if len(names(t, directory)) != 1 {
		t.Fatal("staging file remained")
	}
}

func TestPublicationSetsOrKeepsPermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix permissions")
	}
	directory := t.TempDir()
	path := filepath.Join(directory, "file")
	for _, test := range []struct {
		permissions artifacts.Permissions
		want        os.FileMode
	}{{artifacts.Private, 0o600}, {artifacts.Executable, 0o755}, {artifacts.Keep, 0o640}} {
		mode := artifacts.CreateNew
		switch test.permissions {
		case artifacts.Keep:
			path = filepath.Join(directory, "file")
			must(t, os.Chmod(path, 0o640))
			mode = artifacts.Replace
		case artifacts.Executable:
			path = filepath.Join(directory, "tool")
		}
		must(
			t,
			artifacts.PublishBytes(
				t.Context(),
				path,
				body,
				artifacts.Publication{Mode: mode, Permissions: test.permissions},
			),
		)
		info, err := os.Stat(path)
		must(t, err)
		if info.Mode().Perm() != test.want {
			t.Fatalf("got %o, want %o", info.Mode().Perm(), test.want)
		}
	}
}

func TestStagedDirectoryReplacesInstallation(t *testing.T) {
	root := t.TempDir()
	destination, staged := filepath.Join(root, "1.0.0"), filepath.Join(root, "stage")
	must(t, os.Mkdir(destination, 0o755))
	must(t, os.Mkdir(staged, 0o755))
	must(t, os.WriteFile(filepath.Join(destination, "old"), body, 0o600))
	must(t, os.WriteFile(filepath.Join(staged, "new"), body, 0o600))
	must(t, artifacts.PublishDirectory(t.Context(), staged, destination))
	contents(t, filepath.Join(destination, "new"), body)
	if got := names(t, destination); len(got) != 1 || got[0] != "new" {
		t.Fatal(got)
	}
	if _, err := os.Stat(staged); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
}

func TestReleasePublishedWholeOnceAndRefusesOtherContents(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "tool")
	must(t, os.WriteFile(source, []byte("tool v1"), 0o600))
	data := filepath.Join(root, "data")
	must(t, os.WriteFile(data, []byte("data"), 0o600))
	files := []artifacts.ReleaseFile{
		{Source: source, Path: "target/tool", Digest: declared([]byte("tool v1")), Executable: true},
		{Source: data, Path: "data", Digest: declared([]byte("data"))},
	}
	record := artifacts.ReleaseRecord{Name: "descriptor.json", Bytes: []byte("{\"version\":\"1\"}\n")}
	releases := filepath.Join(root, "releases")
	directory := filepath.Join(releases, "tool-1")
	must(t, artifacts.PublishRelease(t.Context(), directory, record, files))
	contents(t, filepath.Join(directory, record.Name), record.Bytes)
	contents(t, filepath.Join(directory, "target/tool"), []byte("tool v1"))
	contents(t, filepath.Join(directory, "data"), []byte("data"))
	if runtime.GOOS != "windows" {
		for _, file := range files {
			info, err := os.Stat(filepath.Join(directory, file.Path))
			must(t, err)
			want := os.FileMode(0)
			if file.Executable {
				want = 0o111
			}
			if info.Mode().Perm()&0o111 != want {
				t.Fatal("incorrect execute permissions")
			}
		}
	}
	must(t, artifacts.PublishRelease(t.Context(), directory, record, files))
	other := artifacts.ReleaseRecord{Name: record.Name, Bytes: []byte("other")}
	err := artifacts.PublishRelease(t.Context(), directory, other, files)
	if err == nil ||
		!strings.HasSuffix(
			err.Error(),
			string(filepath.Separator)+record.Name+" is already published with other contents",
		) {
		t.Fatal(err)
	}
	must(t, os.WriteFile(filepath.Join(directory, "data"), []byte("corrupt"), 0o600))
	err = artifacts.PublishRelease(t.Context(), directory, record, files)
	if err == nil ||
		!strings.HasSuffix(err.Error(), string(filepath.Separator)+"data is already published with other contents") {
		t.Fatal(err)
	}
	contents(t, filepath.Join(directory, record.Name), record.Bytes)
	must(t, os.WriteFile(source, []byte("tool v2"), 0o600))
	if err := artifacts.PublishRelease(
		t.Context(),
		filepath.Join(releases, "tool-2"),
		record,
		files,
	); !errors.Is(
		err,
		artifacts.ErrDigest,
	) {
		t.Fatal(err)
	}
	if got := names(t, releases); len(got) != 1 || got[0] != "tool-1" {
		t.Fatal(got)
	}
}

func TestInstallLockWaitsAndCanGiveUp(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "artifact.lock")
		held, err := artifacts.AcquireInstallLock(t.Context(), path)
		must(t, err)
		defer func() { must(t, held.Close()) }()
		result := make(chan *artifacts.InstallLock, 1)
		failures := make(chan error, 1)
		before := artifactstest.LockWaits()
		go func() {
			next, err := artifacts.AcquireInstallLock(t.Context(), path)
			failures <- err
			result <- next
		}()
		synctest.Wait()
		if artifactstest.LockWaits() != before+1 {
			t.Fatal("waiter did not wait")
		}
		select {
		case <-result:
			t.Fatal("lock admitted two owners")
		default:
		}
		must(t, held.Close())
		next := <-result
		must(t, <-failures)
		defer func() { must(t, next.Close()) }()
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		go func() {
			lock, err := artifacts.AcquireInstallLock(ctx, path)
			if lock != nil {
				err = errors.Join(err, lock.Close())
			}
			failures <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-failures; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		must(t, next.Close())
	})
}

// install runs the runner's download-then-unpack flow with explicit ownership.
func install(
	ctx context.Context,
	c *artifacts.Client,
	root, url string,
	archive artifacts.Archive,
) (entry string, err error) {
	entry, unpacking, err := artifacts.InstallArchive(ctx, root, archive)
	if err != nil || unpacking == nil {
		return entry, err
	}
	defer func() { err = errors.Join(err, unpacking.Close()) }()
	output, err := os.Create(unpacking.ArchivePath())
	if err != nil {
		return "", err
	}
	downloadErr := artifacts.Download(ctx, c, url, archive.Digest, output)
	if err := errors.Join(downloadErr, output.Close()); err != nil {
		return "", err
	}
	return unpacking.Finish(ctx)
}

func TestArchiveInstalledOnceAndCheckedBeforeUse(t *testing.T) {
	data := artifactstest.Zip(t, map[string][]byte{"app/bin/tool": []byte("tool"), "app/data": []byte("data")})
	server := artifactstest.Start(t, map[string]artifactstest.Answer{"/app.zip": artifactstest.OK(data)})
	archive := artifacts.Archive{Digest: declared(data), Entry: "app/bin/tool"}
	root := t.TempDir()
	directory := filepath.Join(root, archive.Digest.SHA256)
	entry, err := artifacts.Installed(t.Context(), directory, archive)
	must(t, err)
	if entry != "" {
		t.Fatal(entry)
	}
	c := client(t)
	var wg sync.WaitGroup
	results := make([]string, 2)
	failures := make([]error, 2)
	for i := range results {
		wg.Go(func() { results[i], failures[i] = install(t.Context(), c, root, server.URL("/app.zip"), archive) })
	}
	wg.Wait()
	for _, err := range failures {
		must(t, err)
	}
	entry = filepath.Join(directory, "app/bin/tool")
	if results[0] != entry || results[1] != entry || server.Requests() != 1 {
		t.Fatalf("results %v, requests %d", results, server.Requests())
	}
	contents(t, entry, []byte("tool"))
	contents(t, filepath.Join(directory, "app/data"), []byte("data"))
	checked, err := artifacts.Installed(t.Context(), directory, archive)
	must(t, err)
	recorded, err := artifacts.Recorded(t.Context(), directory, archive)
	must(t, err)
	if checked != entry || recorded != entry {
		t.Fatal("receipt did not name entry")
	}
	if got := names(
		t,
		root,
	); len(got) != 2 || got[0] != archive.Digest.SHA256 ||
		got[1] != archive.Digest.SHA256+".lock" {
		t.Fatal(got)
	}
	must(t, os.WriteFile(entry, []byte("changed"), 0o600))
	_, err = artifacts.Installed(t.Context(), directory, archive)
	_ = assertError[*artifacts.InstallationError](t, err)
	_, err = install(t.Context(), c, root, server.URL("/app.zip"), archive)
	_ = assertError[*artifacts.InstallationError](t, err)
	if server.Requests() != 1 {
		t.Fatal("corruption caused redownload")
	}
	// Recorded trusts the receipt; Installed above verifies the bytes.
	recorded, err = artifacts.Recorded(t.Context(), directory, archive)
	must(t, err)
	if recorded != entry {
		t.Fatal("receipt no longer names its entry")
	}
	must(t, os.Remove(filepath.Join(directory, artifacts.ReceiptFile)))
	_, err = artifacts.Installed(t.Context(), directory, archive)
	_ = assertError[*artifacts.InstallationError](t, err)
	other := t.TempDir()
	wrong := archive
	wrong.Digest.SHA256 = strings.Repeat("0", 64)
	_, err = install(t.Context(), c, other, server.URL("/app.zip"), wrong)
	if !errors.Is(err, artifacts.ErrDigest) {
		t.Fatal(err)
	}
	lacking := archive
	lacking.Entry = "app/bin/other"
	_, err = install(t.Context(), c, other, server.URL("/app.zip"), lacking)
	assertPrefix(t, err, "the archive cannot be installed: ")
	got := names(t, other)
	if len(got) != 2 || got[0] != wrong.Digest.SHA256+".lock" || got[1] != archive.Digest.SHA256+".lock" {
		t.Fatal(got)
	}
}

func TestArchiveExtractsInsideInstallationAndNamesFiles(t *testing.T) {
	data := artifactstest.Zip(t, map[string][]byte{"app/bin/tool": []byte("tool"), "../escaped": []byte("no")})
	server := artifactstest.Start(t, map[string]artifactstest.Answer{"/app.zip": artifactstest.OK(data)})
	archive := artifacts.Archive{Digest: declared(data), Entry: "app/bin/tool"}
	root := t.TempDir()
	installs := filepath.Join(root, "installs")
	_, err := install(t.Context(), client(t), installs, server.URL("/app.zip"), archive)
	assertPrefix(t, err, "the archive cannot be installed: ")
	if _, err := os.Stat(filepath.Join(root, "escaped")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	if got := names(t, installs); len(got) != 1 || got[0] != archive.Digest.SHA256+".lock" {
		t.Fatal(got)
	}
	file := filepath.Join(root, "app.zip")
	must(t, os.WriteFile(file, artifactstest.Zip(t, map[string][]byte{"app/bin/tool": []byte("tool")}), 0o600))
	for name, want := range map[string]bool{"app/bin/tool": true, "app/bin": false, "app/bin/other": false} {
		found, err := artifacts.ZipHolds(t.Context(), file, name)
		must(t, err)
		if found != want {
			t.Fatalf("ZipHolds(%s) = %v", name, found)
		}
	}
}

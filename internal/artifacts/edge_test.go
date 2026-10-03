package artifacts_test

import (
	"archive/zip"
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
)

// With -race, the child pays the race runtime's one-second shutdown wait.
// A real second process is needed to prove OS lock ownership.
func TestInstallLockAcrossProcesses(t *testing.T) {
	if path := os.Getenv("DEMI_ARTIFACT_LOCK_CHILD"); path != "" {
		ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
		defer cancel()
		done := make(chan error, 1)
		before := artifactstest.LockWaits()
		go func() {
			lock, err := artifacts.AcquireInstallLock(ctx, path)
			if err == nil {
				err = lock.Close()
			}
			done <- err
		}()
		for artifactstest.LockWaits() == before {
			select {
			case err := <-done:
				t.Fatalf("child acquired a lock held by parent: %v", err)
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			default:
				runtime.Gosched()
			}
		}
		_, err := fmt.Fprintln(os.Stdout, "waiting")
		must(t, err)
		must(t, <-done)
		return
	}
	path := filepath.Join(t.TempDir(), "artifact.lock")
	held, err := artifacts.AcquireInstallLock(t.Context(), path)
	must(t, err)
	defer func() { must(t, held.Close()) }()
	executable, err := os.Executable()
	must(t, err)
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, executable, "-test.run=^TestInstallLockAcrossProcesses$")
	command.Env = append(os.Environ(), "DEMI_ARTIFACT_LOCK_CHILD="+path)
	var stderr bytes.Buffer
	command.Stderr = &stderr
	output, err := command.StdoutPipe()
	must(t, err)
	must(t, command.Start())
	defer func() {
		closeErr := held.Close()
		if err := command.Wait(); err != nil {
			t.Errorf("child: %v: %s", err, stderr.String())
		}
		must(t, closeErr)
	}()
	line, err := bufio.NewReader(output).ReadString('\n')
	must(t, err)
	if line != "waiting\n" {
		t.Fatalf("unexpected child output: %s", line)
	}
	must(t, held.Close())
	// EOF is the child's completion event; the context bounds a broken lock.
	_, err = io.Copy(io.Discard, output)
	must(t, err)
}

func TestUnpackedFixtureInstallsAndPreservesSymlinks(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix fixture symlinks")
	}
	source := t.TempDir()
	must(t, os.MkdirAll(filepath.Join(source, "app/bin"), 0o755))
	entry := filepath.Join(source, "app/bin/tool")
	must(t, os.WriteFile(entry, body, 0o755))
	must(t, os.Symlink("bin/tool", filepath.Join(source, "app/current")))
	archive := artifacts.Archive{Digest: declared([]byte("fixture identity")), Entry: "app/bin/tool"}
	root := t.TempDir()
	result, err := artifactstest.InstallUnpacked(t.Context(), root, archive, entry)
	must(t, err)
	contents(t, result, body)
	original, err := os.Stat(entry)
	must(t, err)
	linked, err := os.Stat(result)
	must(t, err)
	if !os.SameFile(original, linked) {
		t.Fatal("unpacked fixture was copied instead of linked on the same filesystem")
	}
	installed := filepath.Join(root, archive.Digest.SHA256)
	checked, err := artifacts.Installed(t.Context(), installed, archive)
	must(t, err)
	if checked != result {
		t.Fatal(checked)
	}
	link, err := os.Readlink(filepath.Join(installed, "app/current"))
	must(t, err)
	if link != "bin/tool" {
		t.Fatal(link)
	}
	info, err := os.Stat(result)
	must(t, err)
	if info.Mode().Perm() != 0o755 {
		t.Fatal(info.Mode())
	}
	must(t, os.Chmod(result, 0))
	defer func() { must(t, os.Chmod(result, 0o755)) }()
	// The stored receipt alone must suffice in a private cache. No corrupt
	// bytes are planted to assert that a defect is correct behavior.
	recorded, err := artifacts.Recorded(t.Context(), installed, archive)
	must(t, err)
	if recorded != result {
		t.Fatal(recorded)
	}
}

func TestInvalidReceiptsAreErrorsNotCacheMisses(t *testing.T) {
	archive := artifacts.Archive{Digest: declared(body), Entry: "tool"}
	for _, data := range []string{
		`{}`, `{"archiveHash":"x","entryHash":"y","extra":0}`,
		`{"archiveHash":"x","archiveHash":"x","entryHash":"y"}`,
		`{"archiveHash":null,"entryHash":"y"}`,
		`{"archiveHash":"x","entryHash":"y"} false`,
	} {
		t.Run(data, func(t *testing.T) {
			directory := t.TempDir()
			must(t, os.WriteFile(filepath.Join(directory, artifacts.ReceiptFile), []byte(data), 0o600))
			_, err := artifacts.Recorded(t.Context(), directory, archive)
			_ = assertError[*artifacts.InstallationError](t, err)
		})
	}
}

func TestArchivesRejectSymlinkEscapesAndInvalidInput(t *testing.T) {
	for _, target := range []string{"../../escape", "/tmp/escape"} {
		t.Run(target, func(t *testing.T) {
			var data bytes.Buffer
			writer := zip.NewWriter(&data)
			header := &zip.FileHeader{Name: "app/link"}
			header.SetMode(os.ModeSymlink | 0o777)
			entry, err := writer.CreateHeader(header)
			must(t, err)
			_, err = io.WriteString(entry, target)
			must(t, err)
			must(t, writer.Close())
			archive := artifacts.Archive{Digest: declared(data.Bytes()), Entry: "app/link"}
			root := t.TempDir()
			_, unpacking, err := artifacts.InstallArchive(t.Context(), root, archive)
			must(t, err)
			defer func() { must(t, unpacking.Close()) }()
			must(t, os.WriteFile(unpacking.ArchivePath(), data.Bytes(), 0o600))
			_, err = unpacking.Finish(t.Context())
			_ = assertError[*artifacts.ArchiveError](t, err)
			if got := names(t, root); len(got) != 1 || !strings.HasSuffix(got[0], ".lock") {
				t.Fatal(got)
			}
		})
	}
	for _, entry := range []string{"../outside", "/outside", "app\\tool", "."} {
		_, _, err := artifacts.InstallArchive(
			t.Context(),
			t.TempDir(),
			artifacts.Archive{Digest: declared(body), Entry: entry},
		)
		_ = assertError[*artifacts.ArchiveError](t, err)
	}
	root := t.TempDir()
	archive := artifacts.Archive{Digest: declared(body), Entry: "tool"}
	_, unpacking, err := artifacts.InstallArchive(t.Context(), root, archive)
	must(t, err)
	defer func() { must(t, unpacking.Close()) }()
	must(t, os.WriteFile(unpacking.ArchivePath(), body, 0o600))
	_, err = unpacking.Finish(t.Context())
	_ = assertError[*artifacts.ArchiveError](t, err)
}

type brokenReader struct{ err error }

func (r brokenReader) Read([]byte) (int, error) { return 0, r.err }
func TestFailedAndMidstreamCancelledPublicationKeepsOldFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "artifact")
	must(t, os.WriteFile(path, body, 0o600))
	failure := errors.New("input failed")
	err := artifacts.Publish(t.Context(), path, brokenReader{failure}, artifacts.Publication{Mode: artifacts.Replace})
	if !errors.Is(err, failure) {
		t.Fatal(err)
	}
	contents(t, path, body)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	input := &cancelReader{cancel: cancel}
	err = artifacts.Publish(ctx, path, input, artifacts.Publication{Mode: artifacts.Replace})
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	contents(t, path, body)
	if len(names(t, filepath.Dir(path))) != 1 {
		t.Fatal("staging file leaked")
	}
}

type cancelReader struct{ cancel context.CancelFunc }

func (r *cancelReader) Read(p []byte) (int, error) {
	r.cancel()
	return copy(p, "cancelled"), io.EOF
}

func TestReleaseRejectsEscapesDuplicatesAndCancellation(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "source")
	must(t, os.WriteFile(source, body, 0o600))
	file := artifacts.ReleaseFile{Source: source, Path: "tool", Digest: declared(body)}
	record := artifacts.ReleaseRecord{Name: "record.json", Bytes: []byte("record")}
	for _, name := range []string{"../outside", "/outside", "record.json"} {
		invalid := file
		invalid.Path = name
		err := artifacts.PublishRelease(
			t.Context(),
			filepath.Join(root, "release"),
			record,
			[]artifacts.ReleaseFile{invalid},
		)
		if !errors.Is(err, os.ErrInvalid) {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	err := artifacts.PublishRelease(ctx, filepath.Join(root, "release"), record, []artifacts.ReleaseFile{file})
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	if len(names(t, root)) != 1 {
		t.Fatal("cancelled release created staging")
	}
}

func TestArchivePreservesDirectoryPermissionsAfterExtraction(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix permissions")
	}
	var data bytes.Buffer
	writer := zip.NewWriter(&data)
	header := &zip.FileHeader{Name: "app/"}
	header.SetMode(os.ModeDir | 0o500)
	_, err := writer.CreateHeader(header)
	must(t, err)
	output, err := writer.Create("app/tool")
	must(t, err)
	_, err = output.Write(body)
	must(t, err)
	must(t, writer.Close())
	archive := artifacts.Archive{Digest: declared(data.Bytes()), Entry: "app/tool"}
	root := t.TempDir()
	_, unpacking, err := artifacts.InstallArchive(t.Context(), root, archive)
	must(t, err)
	defer func() { must(t, unpacking.Close()) }()
	must(t, os.WriteFile(unpacking.ArchivePath(), data.Bytes(), 0o600))
	entry, err := unpacking.Finish(t.Context())
	must(t, err)
	directory := filepath.Dir(entry)
	defer func() { must(t, os.Chmod(directory, 0o755)) }()
	contents(t, entry, body)
	info, err := os.Stat(directory)
	must(t, err)
	if info.Mode().Perm() != 0o500 {
		t.Fatalf("directory mode = %o", info.Mode().Perm())
	}
	// Failed verification must still remove a read-only extracted subtree.
	archive.Entry = "missing"
	failedRoot := t.TempDir()
	_, pending, err := artifacts.InstallArchive(t.Context(), failedRoot, archive)
	must(t, err)
	defer func() { must(t, pending.Close()) }()
	must(t, os.WriteFile(pending.ArchivePath(), data.Bytes(), 0o600))
	_, err = pending.Finish(t.Context())
	_ = assertError[*artifacts.ArchiveError](t, err)
	if found := names(t, failedRoot); len(found) != 1 || !strings.HasSuffix(found[0], ".lock") {
		t.Fatal(found)
	}
}

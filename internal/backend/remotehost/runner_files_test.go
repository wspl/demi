package remotehost_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// requireHostCode checks the filesystem or stream refusal at its public boundary.
func requireHostCode(t *testing.T, err error, code string) {
	t.Helper()
	var failure *host.Error
	if !errors.As(err, &failure) || failure.Code != code {
		t.Fatalf("wanted %s, got %v", code, err)
	}
}

// fileNames snapshots directory entries to detect abandoned temporary writes.
func fileNames(t *testing.T, path string) []string {
	t.Helper()
	entries, err := os.ReadDir(path)
	requirePipe(t, err)
	names := make([]string, len(entries))
	for i, entry := range entries {
		names[i] = entry.Name()
	}
	return names
}

func TestRunnerFileContentsWholeRangesAndOutsideMessages(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	h := f.Host()
	home := f.Home()
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	large := patternedBytes(runnerwire.MaxMessageBytes + 3)
	requirePipe(t, os.WriteFile(filepath.Join(home, "large.bin"), large, 0o600))
	read, err := h.FS().ReadFile(t.Context(), filepath.Join(home, "large.bin"))
	requirePipe(t, err)
	if !bytes.Equal(read, large) {
		t.Fatal("large read changed")
	}
	requirePipe(
		t,
		h.FS().
			WriteFile(
				t.Context(),
				filepath.Join(home, "copy/large.bin"),
				host.FileContents{Bytes: large},
				host.WriteOptions{CreateParents: true}),
	)
	read, err = os.ReadFile(filepath.Join(home, "copy/large.bin"))
	requirePipe(t, err)
	if !bytes.Equal(read, large) ||
		!reflect.DeepEqual(fileNames(t, filepath.Join(home, "copy")), []string{"large.bin"}) {
		t.Fatal("large write changed or left temporary file")
	}
	video := patternedBytes(3 * 1024 * 1024)
	path := filepath.Join(home, "video.bin")
	requirePipe(t, os.WriteFile(path, video, 0o600))
	for _, span := range []host.ByteRange{
		{
			Offset: 1024*1024 + 7,
			Length: new(uint64(4096)),
		},
		{Offset: uint64(len(video) - 10)},
		{Length: new(uint64(0))},
	} {
		stream, err := h.FS().ReadStream(t.Context(), path, span)
		requirePipe(t, err)
		data, err := collectPipe(t.Context(), stream)
		requirePipe(t, err)
		requirePipe(t, stream.Close(t.Context()))
		end := uint64(len(video))
		if span.Length != nil {
			end = span.Offset + *span.Length
		}
		if !bytes.Equal(data, video[span.Offset:end]) {
			t.Fatal("range changed", span)
		}
	}
	_, err = h.FS().ReadStream(t.Context(), filepath.Join(home, "missing.bin"), host.ByteRange{})
	requireHostCode(t, err, "ENOENT")
	_, err = h.FS().ReadStream(t.Context(), home, host.ByteRange{})
	requireHostCode(t, err, "EISDIR")
	before := fileNames(t, home)
	err = h.FS().
		WriteFile(
			t.Context(),
			filepath.Join(home, "absent/file"),
			host.FileContents{Bytes: patternedBytes(10)},
			host.WriteOptions{})
	requireHostCode(t, err, "ENOENT")
	if !reflect.DeepEqual(fileNames(t, home), before) {
		t.Fatal("refused write left temporary file")
	}
	requirePipe(t, os.Mkdir(filepath.Join(home, "target"), 0o700))
	requirePipe(t, os.WriteFile(filepath.Join(home, "target/kept"), []byte("kept"), 0o600))
	before = fileNames(t, home)
	if err = h.FS().
		WriteFile(
			t.Context(),
			filepath.Join(home, "target"),
			host.FileContents{Bytes: patternedBytes(2 * 1024 * 1024)},
			host.WriteOptions{}); err == nil {
		t.Fatal("replaced a directory")
	}
	read, err = os.ReadFile(filepath.Join(home, "target/kept"))
	requirePipe(t, err)
	if string(read) != "kept" || !reflect.DeepEqual(fileNames(t, home), before) {
		t.Fatal("refused directory write changed files")
	}
	requirePipe(t, os.WriteFile(filepath.Join(home, "replaced"), []byte("old"), 0o600))
	requirePipe(
		t,
		h.FS().
			WriteFile(
				t.Context(),
				filepath.Join(home, "replaced"),
				host.FileContents{Bytes: []byte("new")},
				host.WriteOptions{}),
	)
	read, err = os.ReadFile(filepath.Join(home, "replaced"))
	requirePipe(t, err)
	if string(read) != "new" {
		t.Fatal("replacement failed")
	}
	requirePipe(t, os.Mkdir(filepath.Join(home, "empty"), 0o700))
	entries, err := h.FS().ReadDir(t.Context(), filepath.Join(home, "empty"))
	requirePipe(t, err)
	if len(entries) != 0 {
		t.Fatal(entries)
	}
	_, err = h.FS().Stat(t.Context(), home+"/"+strings.Repeat("x", runnerwire.MaxMessageBytes))
	var failure *host.Error
	if !errors.As(err, &failure) || failure.Kind != host.TooLarge {
		t.Fatal(err)
	}
	listing := filepath.Join(home, "listing")
	requirePipe(t, os.Mkdir(listing, 0o700))
	name := strings.Repeat("n", 250)
	for i := 0; i <= runnerwire.MaxMessageBytes/len(name); i++ {
		requirePipe(t, os.WriteFile(filepath.Join(listing, fmt.Sprintf("%s%d", name, i)), nil, 0o600))
	}
	_, err = h.FS().ReadDir(t.Context(), listing)
	if !errors.As(err, &failure) || failure.Kind != host.TooLarge {
		t.Fatal(err)
	}
	exists, err := h.FS().Exists(t.Context(), listing)
	requirePipe(t, err)
	if !exists || link.IsClosed() {
		t.Fatal("oversize request killed connection")
	}
}

func TestRunnerReaderLeavingStopsReadAndHostKeepsServing(t *testing.T) {
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Tap: tap.input})
	h := f.Host()
	path := filepath.Join(f.Home(), "long.bin")
	file, err := os.Create(path)
	requirePipe(t, err)
	resizeErr := file.Truncate(64 * 1024 * 1024)
	closeErr := file.Close()
	requirePipe(t, errors.Join(resizeErr, closeErr))
	reader, err := h.ReadPipe(t.Context(), path, host.ByteRange{})
	requirePipe(t, err)
	buf := make([]byte, 65536)
	for received := 0; received <= 1024*1024; {
		n, err := reader.Read(t.Context(), buf)
		requirePipe(t, err)
		received += n
	}
	reader.Fail("the preview moved on")
	tap.find(t, func(message runnerwire.Outbound) bool {
		done, ok := message.(*runnerwire.PipeDone)
		return ok && !done.Ok
	})
	early, err := h.FS().ReadStream(t.Context(), path, host.ByteRange{})
	requirePipe(t, err)
	_, err = early.Read(t.Context(), buf)
	requirePipe(t, err)
	requirePipe(t, early.Close(t.Context()))
	exists, err := h.FS().Exists(t.Context(), path)
	requirePipe(t, err)
	if !exists {
		t.Fatal("host stopped serving")
	}
}

func TestRunnerJobPipesCarryStreamsAndRefusedEndsDoNotBlock(t *testing.T) {
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Tap: tap.input})
	h := f.Host()
	payload := make([]byte, 3*1024*1024)
	for i := range payload {
		payload[i] = 'a' + byte(i*7%26)
	}
	input := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	output := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	writer, err := input.Writer()
	requirePipe(t, err)
	reader, err := output.Reader()
	requirePipe(t, err)
	defer func() { requirePipe(t, reader.Close(context.Background())) }()
	request := startRequest("tr a-z A-Z")
	request.CWD = f.Home()
	request.Env = map[string]string{"PATH": "/usr/bin:/bin"}
	request.Stdin = new(input.WireRef())
	request.Stdout = new(output.WireRef())
	job, err := h.StartJob(t.Context(), request)
	requirePipe(t, err)
	fed := make(chan error, 1)
	go func() {
		err := writer.Write(t.Context(), payload)
		if err == nil {
			writer.End()
		} else {
			writer.Fail(err.Error())
		}
		fed <- err
	}()
	data, err := collectPipe(t.Context(), reader)
	requirePipe(t, err)
	requirePipe(t, <-fed)
	if !bytes.Equal(data, bytes.ToUpper(payload)) {
		t.Fatal("piped output changed")
	}
	end, err := job.End(t.Context())
	requirePipe(t, err)
	if end.Status.Kind != host.ProcessExited || end.Status.ExitCode != 0 || end.Output == nil ||
		end.Output.StdoutBytes != uint64(len(payload)) {
		t.Fatal(end)
	}
	for _, id := range []string{input.ID(), output.ID()} {
		done := tap.pipeDone(t, id)
		if !done.Ok || done.Error != nil {
			t.Fatal(done)
		}
	}
	first := 0
	for _, message := range tap.seen {
		if view, ok := message.(*runnerwire.JobOutput); ok && view.JobID == job.ID() {
			if len(view.Bytes) > runnerwire.JobViewBytes {
				t.Fatal("unbounded output frame")
			}
			if view.Offset < runnerwire.JobViewBytes {
				first += len(view.Bytes)
			}
		}
	}
	if first > runnerwire.JobViewBytes {
		t.Fatal("unbounded first view")
	}
	request.Script = "head -c 2000000 /dev/zero; echo done >&2"
	request.Stdin = nil
	request.Stdout = &runnerwire.PipeRef{ID: "gone", URL: "/api/pipes/gone"}
	head, err := h.StartJob(t.Context(), request)
	requirePipe(t, err)
	end, err = head.End(t.Context())
	requirePipe(t, err)
	if end.Status.Kind != host.ProcessExited || end.Status.ExitCode != 0 || end.Output == nil ||
		end.Output.StdoutBytes != 2000000 {
		t.Fatal(end)
	}
	done := tap.pipeDone(t, "gone")
	if done.Ok || done.Error == nil || !strings.Contains(*done.Error, "404") {
		t.Fatal(done)
	}
	request.Script = "wc -c"
	request.Stdin = &runnerwire.PipeRef{ID: "missing", URL: "/api/pipes/missing"}
	request.Stdout = nil
	count, err := h.StartJob(t.Context(), request)
	requirePipe(t, err)
	var counted []byte
	for {
		chunk, err := count.NextOutput(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		requirePipe(t, err)
		counted = append(counted, chunk.Bytes...)
	}
	_, err = count.End(t.Context())
	requirePipe(t, err)
	if strings.TrimSpace(string(counted)) != "0" || tap.pipeDone(t, "missing").Ok {
		t.Fatal("refused stdin did not close")
	}
}

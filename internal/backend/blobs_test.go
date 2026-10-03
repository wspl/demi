package backend_test

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/webapi"
)

// filesBlob places a fixture in the user's durable blob namespace.
func filesBlob(t *testing.T, harness *backendtest.Harness, user string, data []byte) string {
	t.Helper()
	name := fmt.Sprintf("%x", sha256.Sum256(data))
	directory := filepath.Join(harness.DataDir(), "blobs", user)
	wireMust(t, os.MkdirAll(directory, 0755))
	wireMust(t, os.WriteFile(filepath.Join(directory, name), data, 0644))
	return name
}

func filesStatus(t *testing.T, answer backendtest.Answer, want int) {
	t.Helper()
	if answer.Status != want {
		t.Fatalf("HTTP %d, want %d: %s", answer.Status, want, answer.Body)
	}
}
func filesHeader(t *testing.T, answer backendtest.Answer, name, want string) {
	t.Helper()
	if want == "" && answer.Headers.Values(name) != nil {
		t.Fatalf("%s must be absent: %q", name, answer.Headers.Values(name))
	}
	if got := answer.Headers.Get(name); got != want {
		t.Fatalf("%s = %q, want %q", name, got, want)
	}
}

// Local HTTP and disk only; no vendor calls or runner, normally under one second.
func TestBlobNamespaceInertAndImmutable(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	data := []byte("\x89PNG\r\n\x1a\n not really an image")
	name := filesBlob(t, harness, string(master.User.ID), data)
	read := func(path string, session *backendtest.Session) backendtest.Answer {
		t.Helper()
		a, err := backend.Read(ctx, path, session)
		wireMust(t, err)
		return a
	}
	path := "/api/blobs/" + name
	download := read(path, &master)
	filesStatus(t, download, 200)
	if !bytes.Equal(download.Body, data) {
		t.Fatal("blob bytes changed")
	}
	for name, value := range map[string]string{"cache-control": "private, max-age=31536000, immutable", "vary": "Cookie", "x-content-type-options": "nosniff", "content-type": "application/octet-stream", "content-disposition": "attachment"} {
		filesHeader(t, download, name, value)
	}
	image := read(path+"?type=image%2Fpng", &master)
	filesHeader(t, image, "content-type", "image/png")
	filesHeader(t, image, "content-security-policy", "default-src 'none'; style-src 'unsafe-inline'; sandbox")
	filesHeader(t, image, "content-disposition", "")
	video := read(path+"?type=video/mp4", &master)
	filesHeader(t, video, "content-type", "video/mp4")
	filesHeader(t, video, "content-security-policy", "")
	page := read(path+"?type=text/html", &master)
	filesStatus(t, page, 200)
	filesHeader(t, page, "content-type", "application/octet-stream")
	filesHeader(t, page, "content-disposition", "attachment")
	theirs := filesBlob(t, harness, "someone-else", []byte("their bytes"))
	for _, missing := range []string{theirs, strings.ToUpper(name), "not-a-hash", strings.Repeat("0", 64)} {
		filesRefusal(t, read("/api/blobs/"+missing, &master), 404, webapi.ErrorCodeNotFound)
	}
	filesRefusal(t, read(path, nil), 401, webapi.ErrorCodeUnauthenticated)
	wireMust(t, backend.Close(ctx))
}

// Local HTTP and disk only; checks the byte positions used by video players.
func TestBlobByteRangesForVideo(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	data := make([]byte, 1000)
	for i := range data {
		data[i] = byte(i % 251)
	}
	name := filesBlob(t, harness, string(master.User.ID), data)
	path := "/api/blobs/" + name + "?type=video%2Fmp4"
	read := func(r string) backendtest.Answer {
		t.Helper()
		headers := http.Header{}
		if r != "" {
			headers.Set("Range", r)
		}
		a, err := backend.ReadWith(ctx, path, &master, headers)
		wireMust(t, err)
		return a
	}
	whole := read("")
	filesStatus(t, whole, 200)
	if !bytes.Equal(whole.Body, data) {
		t.Fatal("whole blob changed")
	}
	filesHeader(t, whole, "accept-ranges", "bytes")
	probe := read("bytes=0-1")
	filesStatus(t, probe, 206)
	if !bytes.Equal(probe.Body, data[:2]) {
		t.Fatal("probe bytes changed")
	}
	filesHeader(t, probe, "content-range", "bytes 0-1/1000")
	seek := read("bytes=600-")
	filesStatus(t, seek, 206)
	if !bytes.Equal(seek.Body, data[600:]) {
		t.Fatal("seek bytes changed")
	}
	filesHeader(t, seek, "content-range", "bytes 600-999/1000")
	filesHeader(t, seek, "content-type", "video/mp4")
	filesHeader(t, seek, "cache-control", "private, max-age=31536000, immutable")
	past := read("bytes=1000-")
	filesStatus(t, past, 416)
	if len(past.Body) != 0 {
		t.Fatal("unsatisfiable range has a body")
	}
	filesHeader(t, past, "content-range", "bytes */1000")
	wireMust(t, backend.Close(ctx))
}

package backendtest_test

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// storeBlob stores bytes in the user's namespace where the object store of the
// data directory keeps them, and answers their name. The data directory's layout
// is the contract here (storage.md § Ownership and layout): the blobs a page
// refers to are the ones the backend wrote there.
func storeBlob(t *testing.T, h *backendtest.Harness, user string, bytes []byte) string {
	t.Helper()
	sum := sha256.Sum256(bytes)
	name := hex.EncodeToString(sum[:])
	directory := filepath.Join(h.DataDir(), "blobs", user)
	if err := os.MkdirAll(directory, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, name), bytes, 0o644); err != nil {
		t.Fatal(err)
	}
	return name
}

func headerIs(t *testing.T, answer *backendtest.Answer, name, want string) {
	t.Helper()
	got := answer.Header.Values(name)
	if want == "" {
		if len(got) != 0 {
			t.Fatalf("%s is %q, not absent", name, got)
		}
		return
	}
	if len(got) != 1 || got[0] != want {
		t.Fatalf("%s is %q, not %q", name, got, want)
	}
}

// Cost: one backend, about a second.
func TestABlobIsServedFromTheCallersNamespaceInertAndImmutable(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	content := []byte("\x89PNG\r\n\x1a\n not really an image")
	name := storeBlob(t, h, master.User.ID, content)

	download := b.Get("/api/blobs/"+name, master).Expect(http.StatusOK)
	if string(download.Body) != string(content) {
		t.Fatalf("the blob answers %q", download.Body)
	}
	headerIs(t, download, "Cache-Control", "private, max-age=31536000, immutable")
	headerIs(t, download, "Vary", "Cookie")
	headerIs(t, download, "X-Content-Type-Options", "nosniff")
	headerIs(t, download, "Content-Type", "application/octet-stream")
	headerIs(t, download, "Content-Disposition", "attachment")

	image := b.Get("/api/blobs/"+name+"?type=image%2Fpng", master)
	headerIs(t, image, "Content-Type", "image/png")
	headerIs(t, image, "Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; sandbox")
	headerIs(t, image, "Content-Disposition", "")
	video := b.Get("/api/blobs/"+name+"?type=video/mp4", master)
	headerIs(t, video, "Content-Type", "video/mp4")
	headerIs(t, video, "Content-Security-Policy", "")
	// A type the page does not show in place leaves the blob a download.
	page := b.Get("/api/blobs/"+name+"?type=text/html", master).Expect(http.StatusOK)
	headerIs(t, page, "Content-Type", "application/octet-stream")
	headerIs(t, page, "Content-Disposition", "attachment")

	// Another user's name reaches nothing, and neither does a malformed one.
	theirs := storeBlob(t, h, "someone-else", []byte("their bytes"))
	for _, missing := range []string{theirs, strings.ToUpper(name), "not-a-hash", strings.Repeat("0", 64)} {
		refused := b.Get("/api/blobs/"+missing, master)
		if status, code := refused.Refusal(); status != http.StatusNotFound || code != "not_found" {
			t.Fatalf("%s is %d %s", missing, status, code)
		}
	}
	anonymous := b.Get("/api/blobs/"+name, nil)
	if status, code := anonymous.Refusal(); status != http.StatusUnauthorized || code != "unauthenticated" {
		t.Fatalf("a blob without a session is %d %s", status, code)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestABlobIsServedByByteRangeSoAPlayerCanPlayAndSeekAVideo(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	// Bytes that differ from their neighbours, so a misplaced range shows.
	content := make([]byte, 1000)
	for index := range content {
		content[index] = byte(index % 251)
	}
	name := storeBlob(t, h, master.User.ID, content)
	video := "/api/blobs/" + name + "?type=video%2Fmp4"
	ranged := func(value string) *backendtest.Answer {
		return b.Do(backendtest.Request{Path: video, Session: master, Headers: map[string]string{"Range": value}})
	}

	whole := b.Get(video, master).Expect(http.StatusOK)
	if string(whole.Body) != string(content) {
		t.Fatal("the whole blob differs")
	}
	headerIs(t, whole, "Accept-Ranges", "bytes")
	// Safari asks for the first two bytes before it plays anything.
	probe := ranged("bytes=0-1").Expect(http.StatusPartialContent)
	if string(probe.Body) != string(content[:2]) {
		t.Fatalf("the probe answers %v", probe.Body)
	}
	headerIs(t, probe, "Content-Range", "bytes 0-1/1000")
	seek := ranged("bytes=600-").Expect(http.StatusPartialContent)
	if string(seek.Body) != string(content[600:]) {
		t.Fatal("the seek answers the wrong bytes")
	}
	headerIs(t, seek, "Content-Range", "bytes 600-999/1000")
	headerIs(t, seek, "Content-Type", "video/mp4")
	headerIs(t, seek, "Cache-Control", "private, max-age=31536000, immutable")
	past := ranged("bytes=1000-").Expect(http.StatusRequestedRangeNotSatisfiable)
	if len(past.Body) != 0 {
		t.Fatalf("a range past the end answers %d bytes", len(past.Body))
	}
	headerIs(t, past, "Content-Range", "bytes */1000")
	b.Stop()
}

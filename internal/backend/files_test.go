package backend_test

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

const filesConversation = "5a1d0c3e-8f3a-4c1e-9d2b-7a1c2e3f4a01"

type filesDevice struct {
	ctx     context.Context
	harness *backendtest.Harness
	backend *backendtest.TestBackend
	master  backendtest.Session
	paired  *backendtest.Paired
	root    string
	control *sql.DB
}

// filesRefusal checks both the status and typed code of a file scenario's refusal.
func filesRefusal(t *testing.T, answer backendtest.Answer, status int, code webapi.ErrorCode) {
	t.Helper()
	filesStatus(t, answer, status)
	conversationRefusal(t, answer, code)
}

// filesOnDevice gives the conversation a workspace in a real paired runner's home.
func filesOnDevice(t *testing.T) *filesDevice {
	t.Helper()
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	paired, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, filesConversation)
	root := filepath.Join(paired.Runner.Home(), "work")
	wireMust(t, os.MkdirAll(root, 0o755))
	control, err := h.ControlDatabase(ctx, t)
	wireMust(t, err)
	_, err = control.ExecContext(
		ctx,
		"INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at) "+
			"VALUES ('workspace-1', ?1, ?2, ?3, 'work', 0, 0)",
		s.User.ID,
		paired.ID(),
		root,
	)
	wireMust(t, err)
	_, err = control.ExecContext(
		ctx,
		"UPDATE conversations SET target_kind = 'workspace', target_workspace_id = 'workspace-1' WHERE id = ?1",
		filesConversation,
	)
	wireMust(t, err)
	return &filesDevice{ctx, h, b, s, paired, root, control}
}

func (d *filesDevice) call(t *testing.T, method, route string, headers http.Header, body io.Reader) backendtest.Answer {
	t.Helper()
	response, err := d.backend.Response(
		d.ctx,
		method,
		"/api/conversations/"+filesConversation+route,
		&d.master,
		headers,
		body,
	)
	wireMust(t, err)
	answer, err := backendtest.ReadAnswer(d.ctx, response)
	wireMust(t, err)
	return answer
}

func (d *filesDevice) read(t *testing.T, route string) backendtest.Answer {
	t.Helper()
	return d.call(t, http.MethodGet, route, nil, nil)
}

// TestFileDeleteProtectsHostDirectories uses one paired runner and no model; startup is the principal cost.
func TestFileDeleteProtectsHostDirectories(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	wireMust(t, os.MkdirAll(filepath.Join(d.root, "photos/2024"), 0o755))
	wireMust(t, os.WriteFile(filepath.Join(d.root, "photos/2024/a.jpg"), []byte("a"), 0o644))
	wireMust(t, os.WriteFile(filepath.Join(d.root, "notes.md"), []byte("n"), 0o644))
	remove := func(path string) backendtest.Answer {
		return d.call(t, http.MethodDelete, "/fs?path="+url.QueryEscape(path), nil, nil)
	}
	for _, name := range []string{"notes.md", "photos", "nothing"} {
		filesStatus(t, remove(filepath.Join(d.root, name)), 204)
	}
	entries, err := os.ReadDir(d.root)
	wireMust(t, err)
	if len(entries) != 0 {
		t.Fatalf("files left: %v", entries)
	}
	for _, path := range []string{d.root, d.root + "/..", d.paired.Runner.Home(), "/", strings.ToUpper(d.root)} {
		filesRefusal(t, remove(path), 409, webapi.ErrorCodeProtectedPath)
	}
	info, err := os.Stat(d.root)
	wireMust(t, err)
	if !info.IsDir() {
		t.Fatal("workspace removed")
	}
	filesRefusal(t, remove("photos"), 400, webapi.ErrorCodeInvalidQuery)
	wireMust(t, d.backend.Close(d.ctx))
}

// TestFileAccessRequiresOwnedConversationAndAttachedHost uses two paired runners to verify ownership and
// attachment through HTTP, with no model.
func TestFileAccessRequiresOwnedConversationAndAttachedHost(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	wireMust(t, d.harness.AddUser(d.ctx, "user@example.test", "user-pass-1", webapi.RoleUser))
	other, err := d.backend.Login(d.ctx, "user@example.test", "user-pass-1")
	wireMust(t, err)
	foreign, err := d.backend.Read(d.ctx, "/api/conversations/"+filesConversation+"/fs", &other)
	wireMust(t, err)
	filesRefusal(t, foreign, 404, webapi.ErrorCodeConversationNotFound)
	unknown, err := d.backend.Read(d.ctx, "/api/conversations/not-a-uuid/fs", &d.master)
	wireMust(t, err)
	filesRefusal(t, unknown, 404, webapi.ErrorCodeConversationNotFound)
	ci, err := d.backend.Pair(d.ctx, t, &d.master, "ci")
	wireMust(t, err)
	route := "/hosts/" + string(ci.ID()) + "/fs"
	filesRefusal(t, d.read(t, route), 404, webapi.ErrorCodeHostNotAttached)
	_, err = d.control.ExecContext(
		d.ctx,
		"INSERT INTO conversation_hosts (conversation_id, device_id, name, cwd, attached_at) VALUES (?1, ?2, 'ci', NULL, 0)",
		filesConversation,
		ci.ID(),
	)
	wireMust(t, err)
	listing := d.read(t, route)
	filesStatus(t, listing, 200)
	directory, err := webapi.DecodeDirectory(listing.Body)
	wireMust(t, err)
	if directory.Path != ci.Runner.Home() || directory.Home == nil || *directory.Home != ci.Runner.Home() {
		t.Fatalf("attached host directory: %+v", directory)
	}
	made := filepath.Join(ci.Runner.Home(), "made")
	body, err := contract.EncodeJSON(webapi.CreateDirectory{Path: made})
	wireMust(t, err)
	filesStatus(
		t,
		d.call(
			t,
			http.MethodPost,
			route,
			http.Header{"Content-Type": []string{"application/json"}},
			strings.NewReader(string(body)),
		),
		201,
	)
	info, err := os.Stat(made)
	wireMust(t, err)
	if !info.IsDir() {
		t.Fatal("directory not created")
	}
	_, err = d.control.ExecContext(d.ctx, "UPDATE conversation_hosts SET cwd = ?2 WHERE device_id = ?1", ci.ID(), made)
	wireMust(t, err)
	directory, err = webapi.DecodeDirectory(d.read(t, route).Body)
	wireMust(t, err)
	if directory.Path != made {
		t.Fatalf("cwd = %q", directory.Path)
	}
	directory, err = webapi.DecodeDirectory(d.read(t, "/hosts/"+string(d.paired.ID())+"/fs").Body)
	wireMust(t, err)
	if directory.Path != d.root {
		t.Fatalf("main cwd = %q", directory.Path)
	}
	_, err = d.control.ExecContext(d.ctx, "UPDATE conversations SET archived = 1 WHERE id = ?1", filesConversation)
	wireMust(t, err)
	filesRefusal(t, d.read(t, "/fs"), 409, webapi.ErrorCodeConversationArchived)
	filesRefusal(
		t,
		d.read(t, "/fs/raw?path="+url.QueryEscape(filepath.Join(d.root, "a.txt"))),
		409,
		webapi.ErrorCodeConversationArchived,
	)
	wireMust(t, d.backend.Close(d.ctx))
}

// TestFileShutdownEndsOpenDownload uses one runner and a 64 MiB file to exercise shutdown under transfer
// backpressure.
func TestFileShutdownEndsOpenDownload(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	path := filepath.Join(d.root, "long.mp4")
	wireMust(t, os.WriteFile(path, backendtest.Pattern(64*1024*1024, 0), 0o644))
	response, err := d.backend.Response(
		d.ctx,
		http.MethodGet,
		"/api/conversations/"+filesConversation+"/fs/raw?path="+url.QueryEscape(path),
		&d.master,
		nil,
		nil,
	)
	wireMust(t, err)
	defer func() { wireMust(t, response.Body.Close()) }()
	if response.StatusCode != 200 {
		t.Fatalf("download: %s", response.Status)
	}
	first := make([]byte, 1)
	_, err = io.ReadFull(response.Body, first)
	wireMust(t, err)
	closing, cancel := context.WithTimeout(d.ctx, 20*time.Second)
	defer cancel()
	wireMust(t, d.backend.Close(closing))
	_, err = io.Copy(io.Discard, response.Body)
	if err == nil {
		t.Fatal("shutdown completed rather than cut the open download")
	}
}

// filesGit creates reference commits in the fixture repository.
func filesGit(t *testing.T, d *filesDevice, args ...string) {
	t.Helper()
	cmd := exec.CommandContext(
		d.ctx,
		"git",
		append([]string{"-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"}, args...)...)
	cmd.Dir = d.root
	cmd.Env = append(
		os.Environ(),
		"GIT_AUTHOR_NAME=Test",
		"GIT_AUTHOR_EMAIL=test@example.com",
		"GIT_COMMITTER_NAME=Test",
		"GIT_COMMITTER_EMAIL=test@example.com",
	)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v: %s", args, err, output)
	}
}

// TestFilesWorkingTreeTextListingAndOffline uses a real runner and about 21,000 directory entries to exercise
// the wire size limit.
func TestFilesWorkingTreeTextListingAndOffline(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	outside := d.read(t, "/changes")
	filesStatus(t, outside, 200)
	changes, err := webapi.DecodeWorkingTreeChanges(outside.Body)
	wireMust(t, err)
	if changes.Root != d.root || changes.Repository || len(changes.Files) != 0 {
		t.Fatalf("non-repository: %+v", changes)
	}
	filesGit(t, d, "init", "-q", "-b", "main")
	wireMust(t, os.WriteFile(filepath.Join(d.root, "a.txt"), []byte("1\n2\n"), 0o644))
	filesGit(t, d, "add", ".")
	filesGit(t, d, "commit", "-q", "-m", "first")
	wireMust(t, os.WriteFile(filepath.Join(d.root, "a.txt"), []byte("1\n2\n3\n"), 0o644))
	wireMust(t, os.WriteFile(filepath.Join(d.root, "b.txt"), []byte("new\n"), 0o644))
	wireMust(t, os.WriteFile(filepath.Join(d.root, "blob.bin"), []byte{0, 255, 1}, 0o644))
	changes, err = webapi.DecodeWorkingTreeChanges(d.read(t, "/changes").Body)
	wireMust(t, err)
	if !changes.Repository || changes.Truncated || changes.Head == nil || len(*changes.Head) != 40 {
		t.Fatalf("repository: %+v", changes)
	}
	_, err = hex.DecodeString(*changes.Head)
	wireMust(t, err)
	want := []runnerwire.GitChange{
		{Path: "a.txt", Status: " M", Kind: runnerwire.ChangeKindModified, Added: 1},
		{Path: "b.txt", Status: "??", Kind: runnerwire.ChangeKindAdded, Added: 1},
		{Path: "blob.bin", Status: "??", Kind: runnerwire.ChangeKindAdded},
	}
	if !reflect.DeepEqual(changes.Files, want) {
		t.Fatalf("changes: %+v, want %+v", changes.Files, want)
	}
	for _, c := range []struct {
		path, original, modified string
	}{
		{
			"a.txt",
			"1\n2\n",
			"1\n2\n3\n",
		},
		{
			"b.txt",
			"",
			"new\n",
		},
	} {
		sides, err := webapi.DecodeChangeSides(d.read(t, "/changes/file?path="+c.path).Body)
		wireMust(t, err)
		if sides.Original != c.original || sides.Modified != c.modified {
			t.Fatalf("%s: %+v", c.path, sides)
		}
	}
	filesRefusal(t, d.read(t, "/changes/file?path=blob.bin"), 415, webapi.ErrorCodeNotText)
	filesRefusal(t, d.read(t, "/changes/file?path=../x"), 400, webapi.ErrorCodeInvalidQuery)
	a := filepath.Join(d.root, "a.txt")
	text, err := webapi.DecodeFileText(d.read(t, "/fs/file?path="+url.QueryEscape(a)).Body)
	wireMust(t, err)
	if text.Path != a || text.Text != "1\n2\n3\n" {
		t.Fatalf("text: %+v", text)
	}
	filesRefusal(
		t,
		d.read(t, "/fs/file?path="+url.QueryEscape(filepath.Join(d.root, "nope"))),
		404,
		webapi.ErrorCodeFsError,
	)
	body, err := contract.EncodeJSON(webapi.CreateDirectory{Path: filepath.Join(d.root, "made/deep")})
	wireMust(t, err)
	filesStatus(
		t,
		d.call(t, "POST", "/fs", http.Header{"Content-Type": []string{"application/json"}}, bytes.NewReader(body)),
		201,
	)
	info, err := os.Stat(filepath.Join(d.root, "made/deep"))
	wireMust(t, err)
	if !info.IsDir() {
		t.Fatal("nested directory not made")
	}
	listing, err := webapi.DecodeDirectory(d.read(t, "/fs").Body)
	wireMust(t, err)
	if listing.Path != d.root || listing.Home == nil || *listing.Home != d.paired.Runner.Home() {
		t.Fatalf("listing: %+v", listing)
	}
	names := make([]string, 0, len(listing.Entries))
	for _, entry := range listing.Entries {
		names = append(names, entry.Name)
	}
	sort.Strings(names)
	if !reflect.DeepEqual(names, []string{".git", "a.txt", "b.txt", "blob.bin", "made"}) {
		t.Fatalf("entries: %v", names)
	}
	crowded := filepath.Join(d.root, "crowded")
	wireMust(t, os.Mkdir(crowded, 0o755))
	for i := 0; i <= runnerwire.MaxMessageBytes/200; i++ {
		wireMust(t, os.WriteFile(filepath.Join(crowded, fmt.Sprintf("%s%d", strings.Repeat("n", 200), i)), nil, 0o644))
	}
	filesRefusal(t, d.read(t, "/fs?path="+url.QueryEscape(crowded)), 413, webapi.ErrorCodeDirectoryTooLarge)
	filesStatus(t, d.read(t, "/fs/file?path="+url.QueryEscape(a)), 200)
	wireMust(t, d.paired.Runner.Stop(d.ctx))
	wireMust(t, d.backend.UntilOnline(d.ctx, &d.master, d.paired.ID(), false))
	filesRefusal(t, d.read(t, "/changes"), 409, webapi.ErrorCodeDeviceOffline)
	wireMust(t, d.backend.Close(d.ctx))
}

// TestFilesRawRangesInertHeadersAndCommittedSide uses one runner to stream beyond its message limit and read
// committed Git bytes.
func TestFilesRawRangesInertHeadersAndCommittedSide(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	raw := func(path string) string { return "/fs/raw?path=" + url.QueryEscape(filepath.Join(d.root, path)) }
	image := backendtest.Pattern(300000, 0)
	wireMust(t, os.WriteFile(filepath.Join(d.root, "logo.svg"), image, 0o644))
	whole := d.read(t, raw("logo.svg"))
	filesStatus(t, whole, 200)
	for name, value := range map[string]string{
		"content-type":            "image/svg+xml",
		"content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; sandbox",
		"x-content-type-options":  "nosniff",
		"x-accel-buffering":       "no",
		"cache-control":           "private, no-cache",
		"accept-ranges":           "bytes",
	} {
		filesHeader(t, whole, name, value)
	}
	etag := whole.Headers.Get("ETag")
	parts := strings.Split(strings.TrimSuffix(strings.TrimPrefix(etag, "W/\""), "\""), "-")
	if !strings.HasPrefix(etag, "W/\"") || !strings.HasSuffix(etag, "\"") || len(parts) != 2 {
		t.Fatalf("etag: %q", etag)
	}
	for _, part := range parts {
		if part == "" || strings.Trim(part, "0123456789abcdefABCDEF") != "" {
			t.Fatalf("etag: %q", etag)
		}
	}
	if !bytes.Equal(whole.Body, image) {
		t.Fatal("image bytes changed")
	}
	head := d.call(t, "HEAD", raw("logo.svg"), nil, nil)
	filesStatus(t, head, 200)
	filesHeader(t, head, "content-length", "300000")
	filesHeader(t, head, "etag", etag)
	if head.Headers.Get("last-modified") == "" {
		t.Fatal("no last-modified")
	}
	part := d.call(t, "GET", raw("logo.svg"), http.Header{"Range": []string{"bytes=1000-1999"}}, nil)
	filesStatus(t, part, 206)
	filesHeader(t, part, "content-range", "bytes 1000-1999/300000")
	if !bytes.Equal(part.Body, image[1000:2000]) {
		t.Fatal("range shifted")
	}
	filesStatus(t, d.call(t, "GET", raw("logo.svg"), http.Header{"Range": []string{"bytes=300000-"}}, nil), 416)
	filesStatus(t, d.call(t, "GET", raw("logo.svg"), http.Header{"If-None-Match": []string{etag}}, nil), 304)
	version := raw("logo.svg") + "&version=" + url.QueryEscape(etag)
	filesStatus(t, d.read(t, version), 200)
	wireMust(t, os.WriteFile(filepath.Join(d.root, "logo.svg"), backendtest.Pattern(10, 0), 0o644))
	filesRefusal(t, d.read(t, version), 412, webapi.ErrorCodeFileChanged)
	download := d.read(t, raw("logo.svg")+"&download=true")
	filesHeader(t, download, "content-type", "application/octet-stream")
	if !strings.HasPrefix(download.Headers.Get("content-disposition"), `attachment; filename="logo.svg"`) {
		t.Fatal("download filename missing")
	}
	filesHeader(t, download, "content-security-policy", "")
	wireMust(t, os.WriteFile(filepath.Join(d.root, "page.html"), []byte("<script>alert(1)</script>"), 0o644))
	html := d.read(t, raw("page.html"))
	filesHeader(t, html, "content-type", "application/octet-stream")
	if !strings.HasPrefix(html.Headers.Get("content-disposition"), "attachment") {
		t.Fatal("HTML is not a download")
	}
	filesRefusal(t, d.read(t, raw(".")), 404, webapi.ErrorCodeNotFound)
	filesRefusal(t, d.read(t, raw("missing.png")), 404, webapi.ErrorCodeFsError)
	filesRefusal(t, d.read(t, raw("logo.svg")+"&download=1"), 400, webapi.ErrorCodeInvalidQuery)
	video := backendtest.Pattern(3*runnerwire.MaxMessageBytes+5, 0)
	wireMust(t, os.WriteFile(filepath.Join(d.root, "demo.mp4"), video, 0o644))
	streamed := d.read(t, raw("demo.mp4"))
	filesHeader(t, streamed, "content-type", "video/mp4")
	filesHeader(t, streamed, "content-length", "")
	filesHeader(t, streamed, "content-security-policy", "")
	if !bytes.Equal(streamed.Body, video) {
		t.Fatal("video bytes changed")
	}
	filesGit(t, d, "init", "-q", "-b", "main")
	committed := backendtest.Pattern(5000, 0)
	wireMust(t, os.WriteFile(filepath.Join(d.root, "chart.png"), committed, 0o644))
	filesGit(t, d, "add", "chart.png")
	filesGit(t, d, "commit", "-q", "-m", "chart")
	wireMust(t, os.WriteFile(filepath.Join(d.root, "chart.png"), backendtest.Pattern(7, 0), 0o644))
	before := d.read(t, "/changes/raw?path=chart.png")
	filesStatus(t, before, 200)
	filesHeader(t, before, "content-type", "image/png")
	if !bytes.Equal(before.Body, committed) {
		t.Fatal("committed bytes changed")
	}
	tail := d.call(t, "GET", "/changes/raw?path=chart.png", http.Header{"Range": []string{"bytes=-100"}}, nil)
	filesStatus(t, tail, 206)
	if !bytes.Equal(tail.Body, committed[len(committed)-100:]) {
		t.Fatal("committed tail shifted")
	}
	filesHeader(t, d.call(t, "HEAD", "/changes/raw?path=chart.png", nil, nil), "content-length", "5000")
	saved := d.read(t, "/changes/raw?path=chart.png&download=true")
	filesHeader(t, saved, "content-type", "application/octet-stream")
	if !strings.HasPrefix(saved.Headers.Get("content-disposition"), `attachment; filename="chart.png"`) ||
		!bytes.Equal(saved.Body, committed) {
		t.Fatal("committed download changed")
	}
	filesStatus(t, d.read(t, "/changes/raw?path=new.png"), 404)
	filesRefusal(t, d.read(t, "/changes/raw?path=../escape.png"), 400, webapi.ErrorCodeInvalidQuery)
	wireMust(t, os.WriteFile(filepath.Join(d.root, "poster.png"), backendtest.Pattern(8*1024*1024+1, 0), 0o644))
	filesGit(t, d, "add", "poster.png")
	filesGit(t, d, "commit", "-q", "-m", "poster")
	filesRefusal(t, d.read(t, "/changes/raw?path=poster.png"), 413, webapi.ErrorCodeFileTooLarge)
	filesStatus(t, d.call(t, "HEAD", "/changes/raw?path=poster.png", nil, nil), 413)
	wireMust(t, d.backend.Close(d.ctx))
}

// TestFileUploadIsWholeAndRequiresOverwriteConsent uses one runner and a streamed 13 MiB upload, followed by a
// canceled partial copy.
func TestFileUploadIsWholeAndRequiresOverwriteConsent(t *testing.T) {
	t.Parallel()
	d := filesOnDevice(t)
	put := func(path, replace string, body io.Reader) backendtest.Answer {
		route := "/fs/raw?path=" + url.QueryEscape(filepath.Join(d.root, path))
		if replace != "" {
			route += "&replace=" + replace
		}
		return d.call(t, "PUT", route, nil, body)
	}
	read := func(path, want string) {
		t.Helper()
		got, err := os.ReadFile(filepath.Join(d.root, path))
		wireMust(t, err)
		if string(got) != want {
			t.Fatalf("%s = %q, want %q", path, got, want)
		}
	}
	filesStatus(t, put("notes.md", "", strings.NewReader("first")), 204)
	read("notes.md", "first")
	filesRefusal(t, put("notes.md", "", strings.NewReader("second")), 409, webapi.ErrorCodeFileExists)
	read("notes.md", "first")
	filesStatus(t, put("notes.md", "true", strings.NewReader("second")), 204)
	read("notes.md", "second")
	filesStatus(t, put("empty.txt", "", strings.NewReader("")), 204)
	read("empty.txt", "")
	wireMust(t, os.Mkdir(filepath.Join(d.root, "docs"), 0o755))
	filesRefusal(t, put("docs", "true", strings.NewReader("x")), 409, webapi.ErrorCodeIsDirectory)
	filesRefusal(t, put("missing/file.txt", "", strings.NewReader("x")), 404, webapi.ErrorCodeFsError)
	filesRefusal(t, put("notes.md", "yes", strings.NewReader("x")), 400, webapi.ErrorCodeInvalidQuery)
	block := backendtest.Pattern(1024*1024, 0)
	chunks := 3*runnerwire.MaxMessageBytes/len(block) + 1
	readers := make([]io.Reader, chunks)
	for i := range readers {
		readers[i] = bytes.NewReader(block)
	}
	filesStatus(t, put("large.bin", "", io.MultiReader(readers...)), 204)
	written, err := os.ReadFile(filepath.Join(d.root, "large.bin"))
	wireMust(t, err)
	if len(written) != chunks*len(block) || !bytes.Equal(written[len(written)-len(block):], block) {
		t.Fatal("large upload changed")
	}
	before, err := os.ReadDir(d.root)
	wireMust(t, err)
	names := make(map[string]bool, len(before))
	for _, entry := range before {
		names[entry.Name()] = true
	}
	watcher, err := fsnotify.NewWatcher()
	wireMust(t, err)
	defer func() { wireMust(t, watcher.Close()) }()
	wireMust(t, watcher.Add(d.root))
	partial := func() []int64 {
		t.Helper()
		entries, err := os.ReadDir(d.root)
		wireMust(t, err)
		var sizes []int64
		for _, entry := range entries {
			if names[entry.Name()] {
				continue
			}
			info, err := entry.Info()
			if errors.Is(err, os.ErrNotExist) {
				continue
			}
			wireMust(t, err)
			sizes = append(sizes, info.Size())
		}
		return sizes
	}
	reader, writer := io.Pipe()
	ctx, cancel := context.WithCancel(d.ctx)
	done := make(chan struct{})
	var responseErr error
	go func() {
		response, err := d.backend.Response(
			ctx,
			"PUT",
			"/api/conversations/"+filesConversation+"/fs/raw?path="+url.QueryEscape(
				filepath.Join(d.root, "notes.md"),
			)+"&replace=true",
			&d.master,
			nil,
			reader,
		)
		if response != nil {
			err = errors.Join(err, response.Body.Close())
		}
		responseErr = err
		close(done)
	}()
	// The request reader is closed on all exits; join the request before cleanup.
	defer func() {
		cancel()
		closeErr := writer.Close()
		<-done
		wireMust(t, closeErr)
	}()
	_, err = writer.Write(block)
	wireMust(t, err)
	for {
		received := false
		for _, size := range partial() {
			received = received || size > 0
		}
		if received {
			break
		}
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case err := <-watcher.Errors:
			wireMust(t, err)
		case <-watcher.Events:
		}
	}
	wireMust(t, writer.CloseWithError(errors.New("the user's browser went away")))
	<-done
	if responseErr == nil {
		t.Fatal("cut upload received an answer")
	}
	for len(partial()) != 0 {
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case err := <-watcher.Errors:
			wireMust(t, err)
		case <-watcher.Events:
		}
	}
	read("notes.md", "second")
	wireMust(t, d.backend.Close(d.ctx))
}

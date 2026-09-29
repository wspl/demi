package backendtest_test

import (
	"bytes"
	"errors"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

const filesConversation = "5a1d0c3e-8f3a-4c1e-9d2b-7a1c2e3f4a01"

// maxRunnerMessage is the largest frame either end of the runner wire sends.
const maxRunnerMessage = 4 * 1024 * 1024

// onDevice is a conversation whose work runs in root, a directory in the home of
// a paired device's real runner, which a workspace names, as the workspace
// routes make it.
type onDevice struct {
	t      *testing.T
	h      *backendtest.Harness
	b      *backendtest.Backend
	master *backendtest.Session
	paired *backendtest.Paired
	root   string
}

func startOnDevice(t *testing.T, options ...backendtest.Option) *onDevice {
	t.Helper()
	h := backendtest.New(t, options...)
	b, master := h.StartSetUp()
	paired := b.Pair(master, "laptop")
	b.CreateConversation(master, filesConversation)
	root := filepath.Join(paired.Home(), "work")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	workspace := createWorkspace(t, b, master, backendtest.Map{"kind": "device", "deviceId": paired.ID(), "path": root, "name": "work"})
	target := backendtest.Map{"target": backendtest.Map{"kind": "workspace", "workspaceId": workspace}}
	b.Patch("/api/conversations/"+filesConversation, master, target).Expect(http.StatusOK)
	return &onDevice{t: t, h: h, b: b, master: master, paired: paired, root: root}
}

// path is relative in the conversation's directory.
func (d *onDevice) path(relative string) string {
	return filepath.Join(d.root, relative)
}

func (d *onDevice) write(relative string, content []byte) {
	d.t.Helper()
	if err := os.WriteFile(d.path(relative), content, 0o644); err != nil {
		d.t.Fatal(err)
	}
}

// call is a request of the master's to one of the conversation's routes.
func (d *onDevice) call(method, route string, headers map[string]string, body []byte) *backendtest.Answer {
	d.t.Helper()
	request := backendtest.Request{
		Method: method, Path: "/api/conversations/" + filesConversation + route, Session: d.master, Headers: headers,
	}
	if body != nil {
		request.Raw = body
	}
	return d.b.Do(request)
}

func (d *onDevice) get(route string) *backendtest.Answer {
	d.t.Helper()
	return d.call(http.MethodGet, route, nil, nil)
}

// query is the pairs as a query string.
func query(pairs ...string) string {
	values := url.Values{}
	for index := 0; index < len(pairs); index += 2 {
		values.Add(pairs[index], pairs[index+1])
	}
	return values.Encode()
}

func git(t *testing.T, directory string, arguments ...string) {
	t.Helper()
	command := exec.Command("git", append([]string{"-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"}, arguments...)...)
	command.Dir = directory
	command.Env = append(os.Environ(),
		"GIT_AUTHOR_NAME=Test", "GIT_AUTHOR_EMAIL=test@example.com",
		"GIT_COMMITTER_NAME=Test", "GIT_COMMITTER_EMAIL=test@example.com",
	)
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", arguments, err, output)
	}
}

func hasHeader(t *testing.T, answer *backendtest.Answer, name string) bool {
	t.Helper()
	_, present := answer.Header[http.CanonicalHeaderKey(name)]
	return present
}

// Cost: one backend and a real runner, several seconds: a listing over the
// runner's message limit needs about 21,000 files in one directory.
func TestTheWorkingTreeListsItsChangesAndReadsOneFileAndAnOfflineDeviceSaysSo(t *testing.T) {
	t.Parallel()
	d := startOnDevice(t)

	// Not a repository yet: an answer, not an error.
	outside := d.get("/changes").Expect(http.StatusOK)
	if outside.Str("root") != d.root || outside.At("repository") != false {
		t.Fatalf("the changes outside a repository are %s", outside.Body)
	}
	backendtest.AssertJSON(t, outside.At("files"), []any{})

	git(t, d.root, "init", "-q", "-b", "main")
	d.write("a.txt", []byte("1\n2\n"))
	git(t, d.root, "add", ".")
	git(t, d.root, "commit", "-q", "-m", "first")
	d.write("a.txt", []byte("1\n2\n3\n"))
	d.write("b.txt", []byte("new\n"))
	d.write("blob.bin", []byte{0, 255, 1})
	changes := d.get("/changes").Expect(http.StatusOK)
	if changes.At("repository") != true || changes.At("truncated") != false {
		t.Fatalf("the changes are %s", changes.Body)
	}
	head := changes.Str("head")
	if len(head) != 40 || strings.Trim(head, "0123456789abcdefABCDEF") != "" {
		t.Fatalf("the head is %q", head)
	}
	backendtest.AssertJSON(t, changes.At("files"), []any{
		backendtest.Map{"path": "a.txt", "status": " M", "kind": "modified", "added": 1, "removed": 0},
		backendtest.Map{"path": "b.txt", "status": "??", "kind": "added", "added": 1, "removed": 0},
		backendtest.Map{"path": "blob.bin", "status": "??", "kind": "added", "added": 0, "removed": 0},
	})

	backendtest.AssertJSON(t, d.get("/changes/file?path=a.txt").Value(), backendtest.Map{"original": "1\n2\n", "modified": "1\n2\n3\n"})
	backendtest.AssertJSON(t, d.get("/changes/file?path=b.txt").Value(), backendtest.Map{"original": "", "modified": "new\n"})
	wantRefusal(t, d.get("/changes/file?path=blob.bin"), http.StatusUnsupportedMediaType, "not_text", "a binary file")
	wantRefusal(t, d.get("/changes/file?path=../x"), http.StatusBadRequest, "invalid_query", "a path that escapes")

	a := d.path("a.txt")
	backendtest.AssertJSON(t, d.get("/fs/file?"+query("path", a)).Value(), backendtest.Map{"path": a, "text": "1\n2\n3\n"})
	wantRefusal(t, d.get("/fs/file?"+query("path", d.path("nope"))), http.StatusNotFound, "fs_error", "a missing file")

	// The listing starts in the conversation's directory and names the device's
	// home; a directory is made with its parents.
	made := d.call(http.MethodPost, "/fs", map[string]string{"Content-Type": "application/json"}, backendtest.Marshal(backendtest.Map{"path": d.path("made/deep")}))
	made.Expect(http.StatusCreated)
	if info, err := os.Stat(d.path("made/deep")); err != nil || !info.IsDir() {
		t.Fatalf("the directory was not made: %v", err)
	}
	listing := d.get("/fs")
	if listing.Str("path") != d.root || listing.Str("home") != d.paired.Home() {
		t.Fatalf("the listing is %s", listing.Body)
	}
	var names []string
	entries, _ := listing.At("entries").([]any)
	for _, entry := range entries {
		names = append(names, backendtest.At(entry, "name").(string))
	}
	slices.Sort(names)
	if !slices.Equal(names, []string{".git", "a.txt", "b.txt", "blob.bin", "made"}) {
		t.Fatalf("the entries are %v", names)
	}

	// A listing over the runner's message limit fails that request alone.
	crowded := d.path("crowded")
	if err := os.Mkdir(crowded, 0o755); err != nil {
		t.Fatal(err)
	}
	name := strings.Repeat("n", 200)
	for index := 0; index <= maxRunnerMessage/200; index++ {
		if err := os.WriteFile(filepath.Join(crowded, name+strconv.Itoa(index)), nil, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	wantRefusal(t, d.get("/fs?"+query("path", crowded)), http.StatusRequestEntityTooLarge, "directory_too_large", "a crowded directory")
	d.get("/fs/file?" + query("path", a)).Expect(http.StatusOK)

	// Offline: the routes say so rather than waking anything.
	d.paired.Runner.Stop()
	d.b.UntilOnline(d.master, d.paired.ID(), false)
	wantRefusal(t, d.get("/changes"), http.StatusConflict, "device_offline", "the changes of an offline device")
	d.b.Stop()
}

// Cost: one backend and a real runner, about a second: a 12 MiB video and a 8
// MiB image go through the runner's messages.
func TestTheRawRoutesStreamAFileByRangeUnderInertHeadersAndTheCommittedSideFromGit(t *testing.T) {
	t.Parallel()
	d := startOnDevice(t)
	raw := func(path string, extra ...string) string {
		return "/fs/raw?" + query(append([]string{"path", d.path(path)}, extra...)...)
	}
	image := backendtest.Pattern(300_000, 0)
	d.write("logo.svg", image)

	whole := d.get(raw("logo.svg")).Expect(http.StatusOK)
	for name, want := range map[string]string{
		"Content-Type":            "image/svg+xml",
		"Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'; sandbox",
		"X-Content-Type-Options":  "nosniff",
		"X-Accel-Buffering":       "no",
		"Cache-Control":           "private, no-cache",
		"Accept-Ranges":           "bytes",
	} {
		headerIs(t, whole, name, want)
	}
	etag := whole.Header.Get("Etag")
	tag, ok := strings.CutPrefix(etag, `W/"`)
	tag, ok2 := strings.CutSuffix(tag, `"`)
	size, mtime, ok3 := strings.Cut(tag, "-")
	if !ok || !ok2 || !ok3 || strings.Trim(size+mtime, "0123456789abcdefABCDEF") != "" {
		t.Fatalf("the etag is %q", etag)
	}
	if !bytes.Equal(whole.Body, image) {
		t.Fatal("the image arrived changed")
	}

	head := d.call(http.MethodHead, raw("logo.svg"), nil, nil).Expect(http.StatusOK)
	headerIs(t, head, "Content-Length", "300000")
	headerIs(t, head, "Etag", etag)
	if !hasHeader(t, head, "Last-Modified") {
		t.Fatal("a HEAD has no Last-Modified")
	}

	part := d.call(http.MethodGet, raw("logo.svg"), map[string]string{"Range": "bytes=1000-1999"}, nil).Expect(http.StatusPartialContent)
	headerIs(t, part, "Content-Range", "bytes 1000-1999/300000")
	if !bytes.Equal(part.Body, image[1000:2000]) {
		t.Fatal("the range arrived changed")
	}
	d.call(http.MethodGet, raw("logo.svg"), map[string]string{"Range": "bytes=300000-"}, nil).Expect(http.StatusRequestedRangeNotSatisfiable)

	d.call(http.MethodGet, raw("logo.svg"), map[string]string{"If-None-Match": etag}, nil).Expect(http.StatusNotModified)
	d.get(raw("logo.svg", "version", etag)).Expect(http.StatusOK)
	d.write("logo.svg", backendtest.Pattern(10, 0))
	wantRefusal(t, d.get(raw("logo.svg", "version", etag)), http.StatusPreconditionFailed, "file_changed", "a changed file")

	download := d.get(raw("logo.svg", "download", "true"))
	headerIs(t, download, "Content-Type", "application/octet-stream")
	if !strings.HasPrefix(download.Header.Get("Content-Disposition"), `attachment; filename="logo.svg"`) {
		t.Fatalf("the disposition is %q", download.Header.Get("Content-Disposition"))
	}
	if hasHeader(t, download, "Content-Security-Policy") {
		t.Fatal("a download carries a content security policy")
	}
	d.write("page.html", []byte("<script>alert(1)</script>"))
	html := d.get(raw("page.html"))
	headerIs(t, html, "Content-Type", "application/octet-stream")
	if !strings.HasPrefix(html.Header.Get("Content-Disposition"), "attachment") {
		t.Fatalf("a page is shown in place: %q", html.Header.Get("Content-Disposition"))
	}
	wantRefusal(t, d.get(raw(".")), http.StatusNotFound, "not_found", "a directory")
	wantRefusal(t, d.get(raw("missing.png")), http.StatusNotFound, "fs_error", "a missing file")
	wantRefusal(t, d.get(raw("logo.svg", "download", "1")), http.StatusBadRequest, "invalid_query", "a flag that is none")

	// Far past the runner's message limit, whole and in order; streamed, the
	// answer has no length, which a HEAD reports.
	video := backendtest.Pattern(3*maxRunnerMessage+5, 0)
	d.write("demo.mp4", video)
	streamed := d.get(raw("demo.mp4"))
	headerIs(t, streamed, "Content-Type", "video/mp4")
	if hasHeader(t, streamed, "Content-Length") || hasHeader(t, streamed, "Content-Security-Policy") {
		t.Fatalf("a streamed video carries %v", streamed.Header)
	}
	if !bytes.Equal(streamed.Body, video) {
		t.Fatal("the video arrived changed")
	}

	// The committed side of a change comes from git, by range as well.
	git(t, d.root, "init", "-q", "-b", "main")
	committed := backendtest.Pattern(5_000, 0)
	d.write("chart.png", committed)
	git(t, d.root, "add", "chart.png")
	git(t, d.root, "commit", "-q", "-m", "chart")
	d.write("chart.png", backendtest.Pattern(7, 0))
	before := d.get("/changes/raw?path=chart.png").Expect(http.StatusOK)
	headerIs(t, before, "Content-Type", "image/png")
	if !bytes.Equal(before.Body, committed) {
		t.Fatal("the committed side arrived changed")
	}
	tail := d.call(http.MethodGet, "/changes/raw?path=chart.png", map[string]string{"Range": "bytes=-100"}, nil).Expect(http.StatusPartialContent)
	if !bytes.Equal(tail.Body, committed[len(committed)-100:]) {
		t.Fatal("the tail arrived changed")
	}
	headerIs(t, d.call(http.MethodHead, "/changes/raw?path=chart.png", nil, nil), "Content-Length", "5000")
	saved := d.get("/changes/raw?path=chart.png&download=true")
	headerIs(t, saved, "Content-Type", "application/octet-stream")
	if !strings.HasPrefix(saved.Header.Get("Content-Disposition"), `attachment; filename="chart.png"`) || !bytes.Equal(saved.Body, committed) {
		t.Fatalf("the saved copy is %q", saved.Header.Get("Content-Disposition"))
	}
	d.get("/changes/raw?path=new.png").Expect(http.StatusNotFound)
	wantRefusal(t, d.get("/changes/raw?path=../escape.png"), http.StatusBadRequest, "invalid_query", "a path that escapes")

	// Git's copy is read whole, so one over the runner's 8 MiB is refused.
	d.write("poster.png", backendtest.Pattern(8*1024*1024+1, 0))
	git(t, d.root, "add", "poster.png")
	git(t, d.root, "commit", "-q", "-m", "poster")
	wantRefusal(t, d.get("/changes/raw?path=poster.png"), http.StatusRequestEntityTooLarge, "file_too_large", "an oversized file")
	d.call(http.MethodHead, "/changes/raw?path=poster.png", nil, nil).Expect(http.StatusRequestEntityTooLarge)
	d.b.Stop()
}

// cuttable is a request body that sends its first bytes, then waits to be told
// to fail as a browser that went away does.
type cuttable struct {
	first []byte
	sent  bool
	cut   chan struct{}
}

func (c *cuttable) Read(p []byte) (int, error) {
	if !c.sent {
		c.sent = true
		return copy(p, c.first), nil
	}
	<-c.cut
	return 0, errors.New("the browser went away")
}

// repeated is a request body of the block sent count times.
type repeated struct {
	block []byte
	left  int
	from  int
}

func (r *repeated) Read(p []byte) (int, error) {
	if r.left == 0 {
		return 0, io.EOF
	}
	n := copy(p, r.block[r.from:])
	r.from += n
	if r.from == len(r.block) {
		r.from = 0
		r.left--
	}
	return n, nil
}

// Cost: one backend and a real runner, about a second: 12 MiB stream through
// the runner a chunk at a time.
func TestAnUploadStreamsIntoPlaceWholeAndAsksBeforeItWritesOverAFile(t *testing.T) {
	t.Parallel()
	d := startOnDevice(t)
	put := func(path, replace string, body []byte) *backendtest.Answer {
		pairs := []string{"path", d.path(path)}
		if replace != "" {
			pairs = append(pairs, "replace", replace)
		}
		return d.call(http.MethodPut, "/fs/raw?"+query(pairs...), nil, body)
	}
	read := func(path string) string {
		content, err := os.ReadFile(d.path(path))
		if err != nil {
			t.Fatal(err)
		}
		return string(content)
	}

	put("notes.md", "", []byte("first")).Expect(http.StatusNoContent)
	if read("notes.md") != "first" {
		t.Fatalf("notes.md holds %q", read("notes.md"))
	}
	wantRefusal(t, put("notes.md", "", []byte("second")), http.StatusConflict, "file_exists", "an upload over a file")
	if read("notes.md") != "first" {
		t.Fatalf("the refused upload changed notes.md to %q", read("notes.md"))
	}
	put("notes.md", "true", []byte("second")).Expect(http.StatusNoContent)
	if read("notes.md") != "second" {
		t.Fatalf("notes.md holds %q", read("notes.md"))
	}
	put("empty.txt", "", []byte{}).Expect(http.StatusNoContent)
	if info, err := os.Stat(d.path("empty.txt")); err != nil || info.Size() != 0 {
		t.Fatalf("empty.txt is %v (%v)", info, err)
	}

	if err := os.Mkdir(d.path("docs"), 0o755); err != nil {
		t.Fatal(err)
	}
	wantRefusal(t, put("docs", "true", []byte("x")), http.StatusConflict, "is_directory", "an upload over a directory")
	wantRefusal(t, put("missing/file.txt", "", []byte("x")), http.StatusNotFound, "fs_error", "an upload into a missing directory")
	wantRefusal(t, put("notes.md", "yes", []byte("x")), http.StatusBadRequest, "invalid_query", "a flag that is none")

	// Far past the JSON body limit and the runner's message limit, streamed
	// through the runner a chunk at a time.
	block := backendtest.Pattern(1024*1024, 0)
	chunks := 3*maxRunnerMessage/len(block) + 1
	large := d.b.Do(backendtest.Request{
		Method: http.MethodPut, Session: d.master,
		Path:   "/api/conversations/" + filesConversation + "/fs/raw?" + query("path", d.path("large.bin")),
		Stream: &repeated{block: block, left: chunks},
	})
	large.Expect(http.StatusNoContent)
	written, err := os.ReadFile(d.path("large.bin"))
	if err != nil {
		t.Fatal(err)
	}
	if len(written) != chunks*len(block) || !bytes.Equal(written[len(written)-len(block):], block) {
		t.Fatalf("the upload's end arrived changed (%d bytes)", len(written))
	}

	// A body the browser cuts short while the Host holds part of it: the browser,
	// gone, hears no answer, as the page's own cut does, and the Host drops its
	// partial copy at once and leaves the path as it was.
	listed := func() map[string]int64 {
		entries, err := os.ReadDir(d.root)
		if err != nil {
			t.Fatal(err)
		}
		sizes := map[string]int64{}
		for _, entry := range entries {
			// An entry removed while the directory is read is not there.
			if info, err := entry.Info(); err == nil {
				sizes[entry.Name()] = info.Size()
			}
		}
		return sizes
	}
	before := listed()
	// The sizes of what the upload put beside the files: the partial copy,
	// whatever the Host names it.
	partial := func() []int64 {
		var sizes []int64
		for name, size := range listed() {
			if _, known := before[name]; !known {
				sizes = append(sizes, size)
			}
		}
		return sizes
	}
	cut := make(chan struct{})
	failed := make(chan error, 1)
	go func() {
		_, err := d.b.Send(backendtest.Request{
			Method: http.MethodPut, Session: d.master,
			Path:   "/api/conversations/" + filesConversation + "/fs/raw?" + query("path", d.path("notes.md"), "replace", "true"),
			Stream: &cuttable{first: block, cut: cut},
		})
		failed <- err
	}()
	backendtest.Eventually(t, "the Host holds part of the upload", func() bool {
		return slices.ContainsFunc(partial(), func(size int64) bool { return size > 0 })
	})
	close(cut)
	if err := <-failed; err == nil {
		t.Fatal("the cut upload got an answer")
	}
	backendtest.Eventually(t, "the cut upload leaves no partial copy", func() bool { return len(partial()) == 0 })
	if kept := read("notes.md"); kept != "second" {
		t.Fatalf("the path is as it was, not %d bytes", len(kept))
	}
	d.b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestADeleteTakesAFileOrADirectoryAndRefusesTheDirectoriesTheHostNeeds(t *testing.T) {
	t.Parallel()
	d := startOnDevice(t)
	remove := func(path string) *backendtest.Answer {
		return d.call(http.MethodDelete, "/fs?"+query("path", path), nil, nil)
	}
	if err := os.MkdirAll(d.path("photos/2024"), 0o755); err != nil {
		t.Fatal(err)
	}
	d.write("photos/2024/a.jpg", []byte("a"))
	d.write("notes.md", []byte("n"))

	remove(d.path("notes.md")).Expect(http.StatusNoContent)
	remove(d.path("photos")).Expect(http.StatusNoContent)
	remove(d.path("nothing")).Expect(http.StatusNoContent)
	if entries, err := os.ReadDir(d.root); err != nil || len(entries) != 0 {
		t.Fatalf("the directory holds %v (%v)", entries, err)
	}

	for _, path := range []string{d.root, d.path(".."), d.paired.Home(), "/", strings.ToUpper(d.root)} {
		wantRefusal(t, remove(path), http.StatusConflict, "protected_path", path)
	}
	if info, err := os.Stat(d.root); err != nil || !info.IsDir() {
		t.Fatalf("the conversation's directory went: %v", err)
	}
	wantRefusal(t, remove("photos"), http.StatusBadRequest, "invalid_query", "a relative path")
	d.b.Stop()
}

// Cost: one backend, two real runners, a scripted vendor and a real device
// installing the builtin package, a few seconds.
func TestTheHostAccessReachesOnlyTheCallersConversationAndTheHostsBoundToIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	d := startOnDevice(t, backendtest.WithBuiltin())
	b := d.b

	// Another user's conversation answers as a missing one, as an id that is
	// none does.
	other := b.CreateUser(d.master, "user@example.test", "user-pass-1", "user")
	wantRefusal(t, b.Get("/api/conversations/"+filesConversation+"/fs", other), http.StatusNotFound, "conversation_not_found", "another user's conversation")
	wantRefusal(t, b.Get("/api/conversations/not-a-uuid/fs", d.master), http.StatusNotFound, "conversation_not_found", "an id that is none")

	// A device the conversation does not reach is refused. Attached, it is listed
	// from its home until a shell there ended somewhere.
	ci := b.Pair(d.master, "ci")
	onCI := "/hosts/" + ci.ID() + "/fs"
	wantRefusal(t, d.get(onCI), http.StatusNotFound, "host_not_attached", "a device the conversation does not reach")
	b.Post("/api/conversations/"+filesConversation+"/hosts", d.master, backendtest.Map{"deviceId": ci.ID()}).Expect(http.StatusCreated)
	listing := d.get(onCI).Expect(http.StatusOK)
	if listing.Str("path") != ci.Home() || listing.Str("home") != ci.Home() {
		t.Fatalf("the attached host lists %s", listing.Body)
	}
	madeOnCI := filepath.Join(ci.Home(), "made")
	created := d.call(http.MethodPost, onCI, map[string]string{"Content-Type": "application/json"}, backendtest.Marshal(backendtest.Map{"path": madeOnCI}))
	created.Expect(http.StatusCreated)
	if info, err := os.Stat(madeOnCI); err != nil || !info.IsDir() {
		t.Fatalf("the directory was not made: %v", err)
	}
	// A shell on the attached host that ends in a directory leaves the host
	// there: the model runs one through demi host shell.
	provider := b.Anthropic(d.master, vendor, "/work")
	work := b.Open(d.master, vendor, filesConversation, provider, "/work")
	work.Turn(backendtest.ShellCall("cd", `demi host shell --host ci "cd made"`, 20*time.Second), backendtest.Say("moved"))
	work.Socket.Close()
	backendtest.AssertJSON(t, d.get(onCI).At("path"), madeOnCI)
	// Named, the main device is the main Host.
	main := d.get("/hosts/" + d.paired.ID() + "/fs")
	if main.Str("path") != d.root {
		t.Fatalf("the main host lists %s", main.Body)
	}

	// An archived conversation refuses every Host operation, transfers too.
	b.Patch("/api/conversations/"+filesConversation, d.master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	wantRefusal(t, d.get("/fs"), http.StatusConflict, "conversation_archived", "a listing of an archived conversation")
	wantRefusal(t, d.get("/fs/raw?"+query("path", d.path("a.txt"))), http.StatusConflict, "conversation_archived", "a download of an archived conversation")
	b.Stop()
}

// Cost: one backend and a real runner, about a second, and 64 MiB written.
func TestAShutdownEndsAnOpenDownloadInsteadOfWaitingForIt(t *testing.T) {
	t.Parallel()
	d := startOnDevice(t)
	d.write("long.mp4", backendtest.Pattern(64*1024*1024, 0))
	route := "/api/conversations/" + filesConversation + "/fs/raw?" + query("path", d.path("long.mp4"))
	playing, err := d.b.Send(backendtest.Request{Path: route, Session: d.master})
	if err != nil {
		t.Fatal(err)
	}
	defer playing.Body.Close()
	if playing.StatusCode != http.StatusOK {
		t.Fatalf("the download answers %d", playing.StatusCode)
	}
	if n, err := playing.Body.Read(make([]byte, 1)); n == 0 || err != nil {
		t.Fatalf("the download sends nothing: %v", err)
	}

	// The page reads nothing more. The shutdown ends the transfer, which holds
	// the conversation's file gate, and goes on. Stop fails the test when the
	// shutdown waits for the download.
	d.b.Stop()
	// The download was cut short, never completed.
	_, err = io.Copy(io.Discard, playing.Body)
	if err == nil {
		t.Fatal("the download completed")
	}
}

//go:build acceptance

package browser

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"image/png"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/contract"
)

func TestDownloadsPublishCompleteFilesAndSupportMedia(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "download.html")
	check := func(args string, content string) string {
		t.Helper()
		result := f.command(t, tab, "download", args)
		path, err := contract.Decode[string](observedField(t, result, "path"))
		if err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(path)
		if err != nil || (content == "<svg" && !strings.Contains(string(data), content)) ||
			(content != "<svg" && string(data) != content) {
			t.Fatalf("download %q: %s %v", path, data, err)
		}
		if content != "<svg" {
			expectValue(t, observedField(t, result, "bytes"), 17)
		}
		return path
	}
	check(`{"css":"#instant","output":"saved.txt"}`, "download fixture\n")
	f.rejects(t, tab, "download", `{"css":"#instant","output":"saved.txt"}`, "output_exists", "")
	check(`{"css":"#instant","output":"saved.txt","overwrite":true}`, "download fixture\n")
	temporary := check(`{"css":"#instant"}`, "download fixture\n")
	if !filepath.IsAbs(temporary) {
		t.Fatal(temporary)
	}
	if err := os.Remove(temporary); err != nil {
		t.Fatal(err)
	}
	xy, err := contract.Decode[string](
		f.eval(
			t,
			tab,
			`(()=>{const r=document.querySelector('#media').getBoundingClientRect();return (r.x+10)+','+(r.y+10)})()`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	check(browserArgs(t, `{"xy":$0,"output":"media.svg"}`, xy), "<svg")
	f.rejects(t, tab, "download", `{"css":"#delayed","output":"late.txt","timeout":100}`, "timeout", "")
	if _, err := os.Stat(filepath.Join(f.root, "late.txt")); !os.IsNotExist(err) {
		t.Fatal(err)
	}
}

func TestUploadAttachesFilesAndCleansChooserObservation(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "upload.html")
	for _, name := range []string{"one.txt", "two.txt"} {
		if err := os.WriteFile(filepath.Join(f.root, name), []byte(name), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	for _, test := range []struct {
		css   string
		files []string
	}{
		{"#single", []string{"one.txt"}}, {"#multiple", []string{"one.txt", "two.txt"}}, {"#choose", []string{"two.txt", "one.txt"}},
	} {
		result := f.command(t, tab, "upload", browserArgs(t, `{"css":$0,"file":$1}`, test.css, test.files))
		expectValue(t, observedField(t, result, "attached"), len(test.files))
		expectValue(t, f.read(t, tab, "#names", "text"), strings.Join(test.files, ","))
	}
	for _, test := range []struct {
		css     string
		files   []string
		code    string
		timeout int
	}{
		{"#single", []string{"one.txt", "two.txt"}, "invalid_input", 2000},
		{"#choose", []string{"one.txt", "missing.txt"}, "io_error", 2000},
		{"#choose", []string{"."}, "invalid_input", 2000},
		{"#disabled", []string{"one.txt"}, "not_actionable", 200},
		{"#no-chooser", []string{"one.txt"}, "timeout", 300},
	} {
		completion, _, stderr := f.result(
			t,
			"upload",
			browserArgs(t, `{"tab":$0,"css":$1,"file":$2,"timeout":$3}`, tab, test.css, test.files, test.timeout),
		)
		failure, err := browserop.DecodeFailureDocument(stderr)
		exit := uint8(1)
		if test.code == "invalid_input" {
			exit = 2
		}
		if err != nil || completion.ExitCode != exit || string(failure.Error.Code) != test.code {
			t.Fatalf("upload: %+v %s %v", completion, stderr, err)
		}
	}
	expectValue(t, f.eval(t, tab, "triggers"), 2)
	job := f.start(t, "upload", browserArgs(t, `{"tab":$0,"css":"#no-chooser","file":["one.txt"]}`, tab))
	f.waitBusy(t, tab)
	job.cancel()
	requireCancelled(t, job.join(t))
	f.command(t, tab, "upload", `{"css":"#choose","file":["one.txt"]}`)
	expectValue(t, f.read(t, tab, "#names", "text"), "one.txt")
}

func TestCommandsOnAnotherTabRunWhileOneTabIsHeld(t *testing.T) {
	f := chromeFixture(t)
	held := f.open(t, "fixture.html")
	other := f.open(t, "fixture.html")
	job := f.start(t, "wait", browserArgs(t, `{"tab":$0,"url":"**/never","timeout":30000}`, held))
	f.waitBusy(t, held)
	expectValue(t, observedField(t, f.command(t, other, "info", `{}`), "tab"), other)
	f.command(t, other, "inspect", `{}`)
	rows := f.tabs(t)
	if len(rows) != 2 || rows[0].ID != held || rows[1].ID != other {
		t.Fatal(rows)
	}
	f.rejects(t, held, "info", `{}`, "tab_busy", "")
	job.cancel()
	requireCancelled(t, job.join(t))
}

func TestAnActionWorksInTheOlderOfTwoOpenTabs(t *testing.T) {
	f := chromeFixture(t)
	older := f.open(t, "popups.html")
	newer := f.open(t, "popups.html")
	expectValue(t, f.eval(t, older, "document.visibilityState"), "hidden")
	expectValue(t, f.eval(t, newer, "document.visibilityState"), "visible")
	f.click(t, older, "#stay")
	expectValue(t, f.eval(t, older, "window.stays"), 1)
	expectValue(t, f.eval(t, older, "document.visibilityState"), "hidden")
}

func TestTheTabListShowsTheTitleAPageHasNow(t *testing.T) {
	f := chromeFixture(t)
	path := filepath.Join(f.root, "titled.html")
	if err := os.WriteFile(
		path,
		[]byte(`<!doctype html><title>First title</title><script>document.title = 'Listed title'</script>`),
		0o600,
	); err != nil {
		t.Fatal(err)
	}
	opened, err := browserop.DecodeOpenResult(
		f.call(t, "open", browserArgs(t, `{"url":$0}`, (&url.URL{Scheme: "file", Path: path}).String())),
	)
	if err != nil {
		t.Fatal(err)
	}
	tab := opened.Tab
	expectValue(t, observedField(t, f.command(t, tab, "info", `{}`), "title"), "Listed title")
	rows := f.tabs(t)
	if len(rows) != 1 || rows[0].Title != "Listed title" {
		t.Fatal(rows)
	}
}

func TestAssetInventoryCoversCrossProcessFramesAndExpiresOnChildNavigation(t *testing.T) {
	f := chromeFixture(t)
	document, err := os.ReadFile("testdata/assets-frames.html")
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html")
		_, _ = w.Write(document)
	}))
	defer server.Close()
	result, err := browserop.DecodeOpenResult(f.call(t, "open", browserArgs(t, `{"url":$0,"load":"load"}`, server.URL)))
	if err != nil {
		t.Fatal(err)
	}
	tab := result.Tab
	inventory := f.command(t, tab, "assets.list", `{}`)
	inline, err := contract.List(observedField(t, inventory, "inlineSvgs"), contract.Decode[json.RawMessage])
	if err != nil || len(inline) != 3 {
		t.Fatalf("%s %v", inventory, err)
	}
	assets, err := contract.List(observedField(t, inventory, "assets"), contract.Decode[json.RawMessage])
	if err != nil || len(assets) != 1 {
		t.Fatalf("%s %v", inventory, err)
	}
	completion, exported, stderr := f.result(
		t,
		"assets.export",
		browserArgs(
			t,
			`{"tab":$0,"inventory":$1,"kind":["image"],"output-dir":"frames"}`,
			tab,
			observedField(t, inventory, "inventory"),
		),
	)
	if completion.ExitCode != 0 {
		failure, err := browserop.DecodeFailureDocument(stderr)
		if err != nil {
			t.Fatal(err)
		}
		if failure.Error.Details != nil && failure.Error.Details.AssetsExportResult != nil {
			manifest, readErr := os.ReadFile(failure.Error.Details.Manifest)
			t.Fatalf("asset export manifest: %s; read error: %v", manifest, readErr)
		}
		t.Fatal(string(stderr))
	}
	files, err := contract.List(observedField(t, exported, "files"), contract.Decode[json.RawMessage])
	if err != nil || len(files) != 4 {
		t.Fatalf("%s %v", exported, err)
	}
	f.mutate(
		t,
		tab,
		`new Promise(resolve=>{const frame=document.querySelector('iframe');frame.onload=()=>resolve(true);frame.src+='?next'})`,
	)
	f.rejects(
		t,
		tab,
		"assets.export",
		browserArgs(
			t,
			`{"inventory":$0,"kind":["image"],"output-dir":"stale"}`,
			observedField(t, inventory, "inventory"),
		),
		"stale_inventory",
		"",
	)
}

func TestFetchClosureAndCancellationReleaseRegisteredTabs(t *testing.T) {
	f := chromeFixture(t)
	retained := f.open(t, "upload.html")
	args := browserArgs(
		t,
		`{"url":$0,"format":"text","timeout":30000}`,
		[]string{f.url + "/assets.html", f.url + "/stall"},
	)
	for _, cancelled := range []bool{false, true} {
		job := f.start(t, "content.fetch", args)
		ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
		var temporary browserop.TabID
		for temporary == "" {
			if err := ctx.Err(); err != nil {
				cancel()
				t.Fatal(err)
			}
			rows := f.tabs(t)
			ready := false
			for _, row := range rows {
				ready = ready || row.URL == f.url+"/assets.html"
			}
			if !ready {
				continue
			}
			for _, row := range rows {
				if row.ID != retained && row.URL != f.url+"/assets.html" {
					if _, ok := row.CreatedBy.(*browserop.BrowserCreatedByTemporary); !ok {
						cancel()
						t.Fatal(row)
					}
					temporary = row.ID
				}
			}
		}
		cancel()
		if cancelled {
			job.cancel()
			requireCancelled(t, job.join(t))
		} else {
			f.command(t, temporary, "close", `{}`)
			result := job.join(t)
			if result.err != nil || result.completion.ExitCode != 0 {
				t.Fatalf("%+v %s", result, result.stderr)
			}
			expectValue(t, observedField(t, result.stdout, "pages", "1", "error", "code"), "tab_not_found")
			expectValue(t, observedField(t, result.stdout, "pages", "0", "title"), "Assets fixture")
		}
		rows := f.tabs(t)
		if len(rows) != 1 || rows[0].ID != retained {
			t.Fatal(rows)
		}
	}
}

func TestClipboardPreservesTextAndSupportsHTMLAndPNG(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "clipboard.html")
	capabilities := f.command(t, tab, "capabilities", `{}`)
	caps, err := browserop.DecodeCapabilitiesResult(capabilities)
	if err != nil {
		t.Fatal(err)
	}
	available := false
	for _, entry := range caps.Capabilities {
		available = available || entry.ID == "clipboard" && entry.Available
	}
	if !available {
		t.Fatal(string(capabilities))
	}
	write := func(mime string, data []byte, want uint8) []byte {
		t.Helper()
		request := invocation("clipboard.write", browserArgs(t, `{"tab":$0,"mime":$1}`, tab, mime), "acceptance")
		if mime == "" {
			request.Request.Args = []byte(browserArgs(t, `{"tab":$0}`, tab))
		}
		request.Request.Cwd = f.root
		source := &fixtureInput{data: make(chan []byte, 1)}
		source.data <- data
		close(source.data)
		request.Input = cmdsdk.NewInput(source)
		completion, stdout, stderr := call(t, f.s, request)
		if completion.ExitCode != want {
			t.Fatalf("clipboard write: %+v %s", completion, stderr)
		}
		return stdout
	}
	expectValue(t, observedField(t, write("", []byte("hello\n\n"), 0), "bytes"), 7)
	read := func() []byte {
		return observedField(t, f.command(t, tab, "clipboard.read", `{"format":"text"}`), "text")
	}
	expectValue(t, read(), "hello\n\n")
	f.click(t, tab, "#read")
	f.eventually(t, tab, `document.querySelector('#text').textContent==='hello\n\n'`)
	expectValue(t, f.read(t, tab, "#text", "text-content"), "hello\n\n")
	write("", []byte{255}, 2)
	expectValue(t, read(), "hello\n\n")
	write("text/html", []byte("<b>bold</b>"), 0)
	html := f.command(t, tab, "clipboard.read", `{"output-dir":"html"}`)
	items, err := contract.List(observedField(t, html, "items"), contract.Decode[json.RawMessage])
	if err != nil {
		t.Fatal(err)
	}
	hasHTML := false
	for _, item := range items {
		hasHTML = hasHTML || string(observedField(t, item, "mimeType")) == `"text/html"`
	}
	if !hasHTML {
		t.Fatal(string(html))
	}
	shot := f.command(t, tab, "screenshot", `{"output":"image.png"}`)
	path, err := contract.Decode[string](observedField(t, shot, "path"))
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	write("image/png", data, 0)
	image := f.command(t, tab, "clipboard.read", `{"output-dir":"png"}`)
	expectValue(t, observedField(t, image, "items", "0", "mimeType"), "image/png")
	path, err = contract.Decode[string](observedField(t, image, "items", "0", "path"))
	if err != nil {
		t.Fatal(err)
	}
	data, err = os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := png.DecodeConfig(bytes.NewReader(data)); err != nil {
		t.Fatal(err)
	}
	write("image/png", []byte("invalid png"), 2)
	f.click(t, tab, "#write")
	expectValue(t, read(), "page copied text")
}

func TestNativeWebMCPValidatesCallsAndInvalidatesChangedDeclarations(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "webmcp.html")
	list := f.command(t, tab, "webmcp.list", `{}`)
	entries, err := contract.List(observedField(t, list, "entries"), contract.Decode[json.RawMessage])
	if err != nil {
		t.Fatal(err)
	}
	hasEcho := false
	for _, entry := range entries {
		hasEcho = hasEcho || string(observedField(t, entry, "name")) == `"echo"`
	}
	if !hasEcho {
		t.Fatal(string(list))
	}
	tools := observedField(t, list, "tools")
	expectValue(t, observedField(t, f.command(t, tab, "webmcp.list", `{}`), "tools"), tools)
	result := f.command(
		t,
		tab,
		"webmcp.call",
		browserArgs(t, `{"tools":$0,"tool":"echo","arguments":$1}`, tools, `{"text":"hello"}`),
	)
	expectValue(t, observedField(t, result, "result"), json.RawMessage(`{"echo":"hello"}`))
	completion, _, stderr := f.result(
		t,
		"webmcp.call",
		browserArgs(t, `{"tab":$0,"tools":$1,"tool":"echo","arguments":$2}`, tab, tools, `{"text":42}`),
	)
	if completion.ExitCode != 2 {
		t.Fatalf("%+v %s", completion, stderr)
	}
	expectValue(t, f.read(t, tab, "#calls", "text"), "1")
	f.click(t, tab, "#replace")
	f.rejects(
		t,
		tab,
		"webmcp.call",
		browserArgs(t, `{"tools":$0,"tool":"echo","arguments":$1}`, tools, `{"text":"again"}`),
		"stale_tools",
		"",
	)
	tools = observedField(t, f.command(t, tab, "webmcp.list", `{}`), "tools")
	f.command(t, tab, "reload", `{}`)
	f.rejects(
		t,
		tab,
		"webmcp.call",
		browserArgs(t, `{"tools":$0,"tool":"echo","arguments":$1}`, tools, `{"text":"after navigation"}`),
		"stale_tools",
		"",
	)
	tools = observedField(t, f.command(t, tab, "webmcp.list", `{}`), "tools")
	job := f.start(t, "webmcp.call", browserArgs(t, `{"tab":$0,"tools":$1,"tool":"wait","arguments":"{}"}`, tab, tools))
	f.get(t, "/wait-typing")
	job.cancel()
	requireCancelled(t, job.join(t))
	expectValue(t, f.read(t, tab, "#cancelled", "text"), "true")
}

func TestCancelledStreamingDownloadNeverPublishesOutput(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	f.mutate(t, tab, `document.body.innerHTML='<a id="stream" href="/stream-download">Download stream</a>'`)
	job := f.start(
		t,
		"download",
		browserArgs(t, `{"tab":$0,"css":"#stream","output":"stream.bin","timeout":30000}`, tab),
	)
	f.get(t, "/wait-typing")
	if _, err := os.Stat(filepath.Join(f.root, "stream.bin")); !os.IsNotExist(err) {
		t.Fatal(err)
	}
	job.cancel()
	requireCancelled(t, job.join(t))
	entries, err := os.ReadDir(f.root)
	if err != nil || len(entries) != 0 {
		t.Fatalf("download residue %v %v", entries, err)
	}
}

func TestFailedNumberDrawFailsOnlyStepNeedingNumber(t *testing.T) {
	f := chromeFixture(t)
	numbers, requests := cmdsdk.NumbersChannel()
	f.s.SetNumbers(numbers)
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan []uint32, 1)
	go func() {
		counts := []uint32{}
		defer func() { done <- counts }()
		for _, answer := range []uint64{1, 0, 0, 101} {
			select {
			case pending := <-requests:
				counts = append(counts, uint32(pending.Request.Count))
				var err error
				if answer == 0 {
					err = errors.New("the backend is unreachable")
				}
				pending.Reply(answer, err)
			case <-ctx.Done():
				return
			}
		}
	}()
	t.Cleanup(func() {
		cancel()
		numbers.Close()
		<-done
	})
	for n := 1; n <= 8; n++ {
		if id := f.open(t, "popups.html"); string(id) != "t"+strconv.Itoa(n) {
			t.Fatal(id)
		}
	}
	refusal := f.failure(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/popups.html"), "browser_unavailable")
	if !strings.Contains(refusal.Error.Message, "the backend is unreachable") {
		t.Fatal(refusal)
	}
	clicked := f.command(t, "t1", "click", `{"css":"#keep"}`)
	if len(observedField(t, clicked, "openedTabs")) != 0 {
		t.Fatal(string(clicked))
	}
	f.eventually(t, "t1", "window.opened.closed")
	rows := f.tabs(t)
	if len(rows) != 8 {
		t.Fatal(rows)
	}
	for i, row := range rows {
		if string(row.ID) != "t"+strconv.Itoa(i+1) {
			t.Fatal(rows)
		}
	}
	if next := f.open(t, "popups.html"); next != "t101" {
		t.Fatal(next)
	}
	counts := <-done
	done <- counts
	expectValue(t, mustBrowserValue(t, counts), []uint32{8, 8, 8, 8})
}

func TestCommandsAnswerReadableTextUnlessJSONAsked(t *testing.T) {
	f := chromeFixture(t)
	path := filepath.Join(f.root, "readable.html")
	if err := os.WriteFile(
		path,
		[]byte(
			`<!doctype html><title>Readable page</title><h1>Welcome back</h1><button>Sign in</button><select aria-label="Country"><option value="JP">Japan</option><option value="SG">Singapore</option></select>`,
		),
		0o600,
	); err != nil {
		t.Fatal(err)
	}
	text := func(name, args string) string {
		t.Helper()
		request := invocation(name, args, "acceptance")
		request.Request.JSON = new(false)
		completion, stdout, stderr := call(t, f.s, request)
		if completion.ExitCode != 0 {
			t.Fatalf("%s: %s", name, stderr)
		}
		return string(stdout)
	}
	url := (&url.URL{Scheme: "file", Path: path}).String()
	opened := text("open", browserArgs(t, `{"url":$0}`, url))
	line, _, _ := strings.Cut(opened, "\n")
	tab, ok := strings.CutPrefix(line, "Tab: ")
	if !ok || !strings.Contains(opened, "\nTitle: Readable page\n") {
		t.Fatal(opened)
	}
	listed := text("tabs", `{}`)
	lines := strings.Split(listed, "\n")
	if len(lines) < 2 || !strings.HasPrefix(lines[1], tab) || !strings.Contains(lines[1], "Readable page") {
		t.Fatal(listed)
	}
	expectValue(t, observedField(t, f.command(t, browserop.TabID(tab), "info", `{}`), "title"), "Readable page")
	reference := regexp.MustCompile(`\[ref=e[0-9]+\]`)
	clicked := text("click", browserArgs(t, `{"tab":$0,"role":"button","name":"Sign in"}`, tab))
	first, _, _ := strings.Cut(clicked, "\n")
	if got := reference.ReplaceAllString(first, "[ref]"); got != `Clicked button "Sign in" [ref].` {
		t.Fatal(got)
	}
	selected := text(
		"select",
		browserArgs(t, `{"tab":$0,"role":"combobox","name":"Country","option-label":["Singapore"]}`, tab),
	)
	if selected != "Selected: Singapore (SG).\n" {
		t.Fatal(selected)
	}
	matched := text("wait", browserArgs(t, `{"tab":$0,"role":"heading","name":"Welcome back","state":"visible"}`, tab))
	if got := reference.ReplaceAllString(
		matched,
		"[ref]",
	); got != "Matched heading \"Welcome back\" [ref]. State: visible.\n" {
		t.Fatal(got)
	}
}

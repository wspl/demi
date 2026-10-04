//go:build acceptance

package browser

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"github.com/wspl/demi/internal/commandwire"
)

type browserFixture struct {
	headers      <-chan http.Header
	conversation string
	env          map[string]string
	s            *service
	root         string
	url          string
	caller       uint64
	locale       *commandwire.CommandLocale
}

// chromeFixture exercises production composition, including the live hub. Each
// scenario costs one real Chrome launch (budget 30 s), unlike the default suite.
func chromeFixture(t *testing.T) *browserFixture {
	t.Helper()
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to the pinned Chrome for Testing executable")
	}
	f := &browserFixture{s: newService(), root: t.TempDir(), conversation: "acceptance"}
	f.s.SetNumbers(cmdsdktest.CountingNumbers(t))
	f.s.SetArtifacts(
		cmdsdktest.ArtifactsFrom(
			t,
			func(_ context.Context, q commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
				return commandwire.ArtifactAnswer{ID: q.ID, Path: &executable}, nil
			},
		),
	)
	headers := make(chan http.Header, 16)
	f.headers = headers
	site := browserSite(t)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/fidelity.html" || r.URL.Path == "/live.html" {
			select {
			case headers <- r.Header.Clone():
			case <-r.Context().Done():
				return
			}
		}
		site.ServeHTTP(w, r)
	}))
	f.url = server.URL
	t.Cleanup(server.Close)
	t.Cleanup(func() {
		if err := f.s.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return f
}

// browserArgs substitutes encoded browser values into a literal contract fixture.
func browserArgs(t *testing.T, template string, values ...any) string {
	t.Helper()
	for i, value := range values {
		raw, err := cdp.Value(value)
		if err != nil {
			t.Fatal(err)
		}
		template = strings.ReplaceAll(template, fmt.Sprintf("$%d", i), string(raw))
	}
	return template
}

func (f *browserFixture) result(t *testing.T, operation, args string) (commandwire.Completion, []byte, []byte) {
	t.Helper()
	request := invocation(operation, args, f.conversation)
	request.Request.Cwd = f.root
	if f.env != nil {
		request.Request.Env = f.env
	}
	if f.caller != 0 {
		request.Request.Context.Caller = &commandwire.AgentCaller{Number: f.caller}
	}
	if f.locale != nil {
		request.Request.Context.Locale = *f.locale
	}
	return call(t, f.s, request)
}

func (f *browserFixture) call(t *testing.T, operation, args string) []byte {
	t.Helper()
	completion, stdout, stderr := f.result(t, operation, args)
	if completion.ExitCode != 0 {
		t.Fatalf("%s: %s", operation, stderr)
	}
	return stdout
}

func (f *browserFixture) failure(t *testing.T, operation, args, code string) browserop.FailureDocument {
	t.Helper()
	completion, _, stderr := f.result(t, operation, args)
	failure, err := browserop.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode == 0 || string(failure.Error.Code) != code {
		t.Fatalf("%s: completion=%+v stderr=%s err=%v", operation, completion, stderr, err)
	}
	return failure
}

func (f *browserFixture) open(t *testing.T, name string) browserop.TabID {
	t.Helper()
	result, err := browserop.DecodeOpenResult(f.call(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/"+name)))
	if err != nil {
		t.Fatal(err)
	}
	return result.Tab
}

func (f *browserFixture) tabs(t *testing.T) []browserop.BrowserTab {
	t.Helper()
	result, err := browserop.DecodeTabsResult(f.call(t, "tabs", `{}`))
	if err != nil {
		t.Fatal(err)
	}
	return result.Tabs
}

func (f *browserFixture) read(t *testing.T, tab browserop.TabID, css, property string) json.RawMessage {
	t.Helper()
	result, err := browserop.DecodeReadResult(
		f.call(t, "read", browserArgs(t, `{"tab":$0,"css":$1,"property":$2}`, tab, css, property)),
	)
	if err != nil {
		t.Fatal(err)
	}
	one, ok := result.(*browserop.ReadResultOne)
	if !ok {
		t.Fatalf("read returned %T", result)
	}
	return one.Value
}

func TestConversationBrowserCommandsShareStateAndRetire(t *testing.T) {
	f := chromeFixture(t)
	if len(f.tabs(t)) != 0 {
		t.Fatal("browser started with tabs")
	}
	tab := f.open(t, "fixture.html")
	f.call(t, "fill", browserArgs(t, `{"tab":$0,"css":"#email","text":"agent@example.test"}`, tab))
	if got := string(f.read(t, tab, "#email", "value")); got != `"agent@example.test"` {
		t.Fatal(got)
	}
	inspected := f.call(t, "inspect", browserArgs(t, `{"tab":$0}`, tab))
	if strings.Contains(string(inspected), "fixture-secret") {
		t.Fatal("password leaked")
	}
	f.failure(t, "eval", browserArgs(t, `{"tab":$0,"expression":"window.sideEffects++"}`, tab), "side_effect_rejected")
	f.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#normal"}`, tab))
	value, err := browserop.DecodeEvalResult(
		f.call(t, "eval", browserArgs(t, `{"tab":$0,"expression":"normalClicks"}`, tab)),
	)
	if err != nil || string(value.Value) != "1" {
		t.Fatalf("%+v %v", value, err)
	}
	f.call(t, "close", browserArgs(t, `{"tab":$0}`, tab))
	if len(f.tabs(t)) != 0 {
		t.Fatal("last tab survived close")
	}
	if got := string(callLifecycle(t, f.s, &commandwire.ConversationQuery{})); got != `{"conversations":[]}` {
		t.Fatal(got)
	}
	next := f.open(t, "fixture.html")
	if next == tab {
		t.Fatal("tab number reused")
	}
	f.failure(t, "info", browserArgs(t, `{"tab":$0}`, tab), "tab_not_found")
	callLifecycle(t, f.s, &commandwire.ConversationRelease{Conversation: "acceptance"})
	if len(f.tabs(t)) != 0 {
		t.Fatal("release retained tabs")
	}
}

func TestAnActionNamesTheTabsItOpened(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "popups.html")
	plain, err := browserop.DecodeActionResult(f.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#stay"}`, tab)))
	if err != nil || plain.OpenedTabs != nil {
		t.Fatalf("plain=%+v err=%v", plain, err)
	}
	opened, err := browserop.DecodeActionResult(f.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#open"}`, tab)))
	if err != nil || opened.OpenedTabs == nil || len(*opened.OpenedTabs) != 1 {
		t.Fatalf("opened=%+v err=%v", opened, err)
	}
	popup := (*opened.OpenedTabs)[0]
	info, err := browserop.DecodeInfoResult(f.call(t, "info", browserArgs(t, `{"tab":$0}`, popup)))
	if err != nil || info.URL != "about:blank" {
		t.Fatalf("info=%+v err=%v", info, err)
	}
	found := false
	for _, row := range f.tabs(t) {
		if row.ID == popup {
			created, ok := row.CreatedBy.(*browserop.BrowserCreatedByPage)
			if !ok || created.Opener != tab {
				t.Fatal(row)
			}
			found = true
		}
	}
	if !found {
		t.Fatal("popup absent from registry")
	}
	again, err := browserop.DecodeActionResult(f.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#stay"}`, tab)))
	if err != nil || again.OpenedTabs != nil {
		t.Fatalf("again=%+v err=%v", again, err)
	}
}

func TestFetchReturnsInputOrderAndReleasesBatchTabs(t *testing.T) {
	f := chromeFixture(t)
	retained := f.open(t, "upload.html")
	urls := []string{f.url + "/assets.html", f.url + "/download.html"}
	result, err := browserop.DecodeContentFetchResult(
		f.call(t, "content.fetch", browserArgs(t, `{"url":$0,"format":"html"}`, urls)),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Pages) != 2 || result.Pages[0].RequestedURL != urls[0] || result.Pages[1].RequestedURL != urls[1] ||
		result.Pages[0].Title != "Assets fixture" ||
		!strings.Contains(result.Pages[1].Content, "Delayed save") {
		t.Fatalf("%+v", result)
	}
	rows := f.tabs(t)
	if len(rows) != 1 || rows[0].ID != retained {
		t.Fatal(rows)
	}
	dom, err := browserop.DecodeContentFetchResult(
		f.call(t, "content.fetch", browserArgs(t, `{"url":$0,"format":"dom"}`, urls[:1])),
	)
	if err != nil || len(dom.Pages) != 1 || dom.Pages[0].Content == "" {
		t.Fatalf("%+v %v", dom, err)
	}
	f.failure(
		t,
		"content.fetch",
		browserArgs(t, `{"url":$0}`, []string{urls[0], "javascript:alert(1)"}),
		"invalid_input",
	)
	if len(f.tabs(t)) != 1 {
		t.Fatal("batch tabs leaked")
	}
}

func TestAssetsExportObservedContentAndKeepPartialSuccess(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "assets.html")
	inventory, err := browserop.DecodeAssetsListResult(f.call(t, "assets.list", browserArgs(t, `{"tab":$0}`, tab)))
	if err != nil {
		t.Fatal(err)
	}
	if len(inventory.Assets) == 0 || len(inventory.InlineSVGs) != 1 {
		t.Fatalf("%+v", inventory)
	}
	result, err := browserop.DecodeAssetsExportResult(
		f.call(
			t,
			"assets.export",
			browserArgs(
				t,
				`{"tab":$0,"inventory":$1,"kind":["image"],"output-dir":"assets"}`,
				tab,
				inventory.Inventory,
			),
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Files) != 2 {
		t.Fatal(result)
	}
	manifest, err := os.ReadFile(result.Manifest)
	if err != nil {
		t.Fatal(err)
	}
	expectValue(t, observedField(t, manifest, "failures"), []string{})
	for _, file := range result.Files {
		data, err := os.ReadFile(file.Path)
		if err != nil || !strings.Contains(string(data), "<svg") {
			t.Fatalf("asset=%s err=%v", data, err)
		}
	}
	if err := os.Mkdir(filepath.Join(f.root, "partial"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(
		filepath.Join(f.root, "partial", string(inventory.Assets[0].ID)+".svg"),
		[]byte("existing"),
		0o600,
	); err != nil {
		t.Fatal(err)
	}
	failure := f.failure(
		t,
		"assets.export",
		browserArgs(t, `{"tab":$0,"inventory":$1,"kind":["image"],"output-dir":"partial"}`, tab, inventory.Inventory),
		"partial_failure",
	)
	if failure.Error.Details == nil || failure.Error.Details.AssetsExportResult == nil ||
		len(failure.Error.Details.Files) != 1 {
		t.Fatal(failure)
	}
	f.call(t, "reload", browserArgs(t, `{"tab":$0}`, tab))
	f.failure(
		t,
		"assets.export",
		browserArgs(t, `{"tab":$0,"inventory":$1,"kind":["image"],"output-dir":"stale"}`, tab, inventory.Inventory),
		"stale_inventory",
	)
}

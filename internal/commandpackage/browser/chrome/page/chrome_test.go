//go:build acceptance

package page_test

import (
	"bytes"
	"encoding/json"
	"image/png"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/page"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

// TestChromePage costs one pinned Chrome launch and a local fixture (budget 30 s).
// It observes actual DOM, native form state, screenshots and Host file transfers;
// scripted CDP replies cannot establish that Chrome applied these operations.
func TestChromePage(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to the pinned Chrome for Testing executable")
	}
	ctx := t.Context()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/download" {
			w.Header().Set("Content-Disposition", `attachment; filename="fixture.txt"`)
			w.Header().Set("Content-Type", "text/plain")
			_, _ = w.Write([]byte("downloaded fixture"))
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write([]byte(`<!doctype html><html><head><title>Page fixture</title></head><body>
<label>Email<input id="email" aria-label="Email"></label>
<label>Date<input id="date" type="date"></label>
<label>Password<input type="password" value="secret"></label>
<label><input id="confirm" type="checkbox">Confirm</label>
<select id="options"><option value="a">Alpha</option><option value="b">Beta</option></select>
<input id="file" type="file"><button id="submit" ` +
			`onclick="document.querySelector('#status').textContent=` +
			`document.querySelector('#email').value+' '+document.querySelector('#date').value+' '+` +
			`document.querySelector('#confirm').checked">Submit</button>
<p id="status">Ready</p><a id="download" href="/download">Download</a>
<svg width="20" height="20"><rect width="20" height="20" fill="red"/></svg>
</body></html>`))
	}))
	defer server.Close()
	environment := tabstest.Launch(
		ctx,
		t,
		tabs.LaunchOptions{
			Executable: executable,
			Locale:     commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
		},
	)
	tab, err := environment.Open(ctx, server.URL, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(15 * time.Second)
	target := func(selector string) browserproto.BrowserTarget {
		return browserproto.BrowserTarget{BrowserQueryMatch: browserproto.BrowserQueryMatch{CSS: &selector}}
	}
	tree, err := page.Inspect(ctx, tab, browserproto.InspectInput{Tab: tab.ID()}, deadline)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := tree.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), "secret") || !strings.Contains(string(encoded), "protected") {
		t.Fatalf("password observation: %s", encoded)
	}
	for _, fill := range []struct{ selector, text string }{
		{
			"#email",
			"discard",
		},
		{
			"#email",
			"",
		},
		{
			"#email",
			"agent@example.test",
		},
		{
			"#date",
			"2026-10-03",
		},
	} {
		if _, err = page.Fill(
			ctx,
			tab,
			browserproto.FillInput{Tab: tab.ID(), BrowserTarget: target(fill.selector), Text: fill.text},
			deadline,
		); err != nil {
			t.Fatal(err)
		}
		if fill.text == "" {
			value, err := page.Evaluate(ctx, tab, `document.querySelector('#email').value`, time.Second)
			if err != nil || string(value) != `""` {
				t.Fatalf("cleared value=%s err=%v", value, err)
			}
		}
	}
	if _, err = page.Check(
		ctx,
		tab,
		browserproto.CheckInput{Tab: tab.ID(), BrowserTarget: target("#confirm"), Value: true},
		deadline,
	); err != nil {
		t.Fatal(err)
	}
	options := []string{"b"}
	if _, err = page.Select(
		ctx,
		tab,
		browserproto.SelectInput{Tab: tab.ID(), BrowserTarget: target("#options"), Value: &options},
		deadline,
	); err != nil {
		t.Fatal(err)
	}
	if _, err = page.Click(
		ctx,
		tab,
		browserproto.ClickInput{Tab: tab.ID(), BrowserTarget: target("#submit")},
		deadline,
	); err != nil {
		t.Fatal(err)
	}
	evaluated, err := page.Evaluate(ctx, tab, `document.querySelector('#status').textContent`, time.Second)
	if err != nil {
		t.Fatal(err)
	}
	var status string
	if err = json.Unmarshal(evaluated, &status); err != nil {
		t.Fatal(err)
	}
	if status != "agent@example.test 2026-10-03 true" {
		t.Fatalf("submitted state: %q", status)
	}
	if _, err = page.Evaluate(
		ctx,
		tab,
		`document.querySelector('#status').textContent='changed'`,
		time.Second,
	); err == nil {
		t.Fatal("read-only evaluation allowed mutation")
	}
	screenshot, err := page.Screenshot(ctx, tab, time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = png.Decode(bytes.NewReader(screenshot)); err != nil {
		t.Fatal(err)
	}
	cwd := t.TempDir()
	file := filepath.Join(cwd, "upload.txt")
	if err = os.WriteFile(file, []byte("upload fixture"), 0o600); err != nil {
		t.Fatal(err)
	}
	invocation := &commandsdk.InvocationContext[commandproto.Invocation]{Request: commandproto.Invocation{Cwd: cwd}}
	uploaded, err := page.Upload(
		ctx,
		invocation,
		tab,
		browserproto.UploadInput{
			Tab:           tab.ID(),
			BrowserTarget: target("#file"),
			File:          []browserproto.LocatorText{browserproto.LocatorText(file)},
		},
		deadline,
	)
	if err != nil || uploaded.Attached != 1 {
		t.Fatalf("upload=%+v err=%v", uploaded, err)
	}
	output := "download.txt"
	downloaded, err := page.Download(
		ctx,
		invocation,
		environment,
		tab,
		browserproto.DownloadInput{Tab: tab.ID(), BrowserTarget: target("#download"), Output: &output},
		deadline,
	)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(downloaded.Path)
	if err != nil || string(data) != "downloaded fixture" {
		t.Fatalf("download=%q err=%v", data, err)
	}
	inventory, err := page.AssetsList(ctx, invocation, tab, browserproto.AssetsListInput{Tab: tab.ID()}, deadline)
	if err != nil {
		t.Fatal(err)
	}
	if len(inventory.InlineSVGs) != 1 {
		t.Fatalf("SVG inventory=%+v", inventory)
	}
}

// These dependency regressions cost one Chrome launch and local HTTP fixture
// (budget 30 s). k-browser's full scenarios additionally exercise command wiring.
func TestChromeRendererUploadsAssetsConsoleAndFailedNavigation(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("DEMI_TEST_CHROME supplies real Chrome")
	}
	ctx := t.Context()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/drop" {
			connection, _, err := http.NewResponseController(w).Hijack()
			if err != nil {
				t.Error(err)
				return
			}
			if err := connection.Close(); err != nil {
				t.Error(err)
			}
			return
		}
		w.Header().Set("Content-Type", "text/html")
		if r.URL.Path == "/child" {
			_, _ = w.Write(
				[]byte(
					`<button id="log" onclick="console.info('child-console')">Log</button><img ` +
						`src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' ` +
						`height='20'%3E%3Crect width='20' height='20'/%3E%3C/svg%3E">`,
				),
			)
			return
		}
		_, _ = w.Write(
			[]byte(
				`<!doctype html><input id="files" type="file" multiple hidden><button id="choose" ` +
					`onclick="document.querySelector('#files').click()">Choose</button><button id="disabled" ` +
					`disabled>Disabled</button><iframe ` +
					`id="child"></iframe><script>document.querySelector('iframe').src=` +
					`location.href.replace('127.0.0.1','localhost')+'child';</script>`,
			),
		)
	}))
	defer server.Close()
	environment := tabstest.Launch(
		ctx,
		t,
		tabs.LaunchOptions{
			Executable: executable,
			Locale:     commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
		},
	)
	tab, err := environment.Open(ctx, server.URL+"/", 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if err := tab.WaitCurrentLoad(ctx, browserproto.LoadLoad); err != nil {
		t.Fatal(err)
	}
	deadline := func() time.Time {
		return time.Now().Add(5 * time.Second)
	}
	cwd := t.TempDir()
	invocation := &commandsdk.InvocationContext[commandproto.Invocation]{Request: commandproto.Invocation{Cwd: cwd}}
	file := filepath.Join(cwd, "chooser.txt")
	if err := os.WriteFile(file, []byte("chooser contents"), 0o600); err != nil {
		t.Fatal(err)
	}
	selector := "#choose"
	upload := browserproto.UploadInput{
		Tab:           tab.ID(),
		BrowserTarget: browserproto.BrowserTarget{BrowserQueryMatch: browserproto.BrowserQueryMatch{CSS: &selector}},
		File:          []browserproto.LocatorText{browserproto.LocatorText(file)},
	}
	result, err := page.Upload(ctx, invocation, tab, upload, deadline())
	if err != nil || result.Attached != 1 {
		t.Fatalf("chooser result=%+v error=%v", result, err)
	}
	value, err := page.Evaluate(ctx, tab, `document.querySelector('#files').files[0].name`, time.Second)
	if err != nil || string(value) != `"chooser.txt"` {
		t.Fatalf("files=%s err=%v", value, err)
	}
	selector = "#disabled"
	_, err = page.Upload(ctx, invocation, tab, upload, time.Now().Add(200*time.Millisecond))
	if cdp.ErrorCode(err) != "not_actionable" {
		t.Fatalf("disabled upload: %v", err)
	}
	inventory, err := page.AssetsList(ctx, invocation, tab, browserproto.AssetsListInput{Tab: tab.ID()}, deadline())
	if err != nil {
		t.Fatal(err)
	}
	output := filepath.Join(cwd, "assets")
	exported, err := page.AssetsExport(
		ctx,
		invocation,
		tab,
		browserproto.AssetsExportInput{
			Tab:       tab.ID(),
			Inventory: inventory.Inventory,
			Kind:      &[]browserproto.AssetKind{browserproto.AssetKindImage},
			OutputDir: output,
		},
		deadline(),
	)
	if err != nil || len(exported.Files) != 1 {
		t.Fatalf("child asset export=%+v error=%v", exported, err)
	}
	snapshot, err := cdp.CaptureFrames(ctx, tab.Page())
	if err != nil {
		t.Fatal(err)
	}
	var child cdp.FrameTarget
	for _, frame := range snapshot.Frames {
		if frame.Target.TargetID() != tab.TargetID() {
			child = frame.Target
		}
	}
	if child == nil {
		t.Fatal("fixture did not create a cross-process renderer")
	}
	_, exception, err := runtime.Evaluate(`document.querySelector('#log').click()`).
		Do(protocol.WithExecutor(ctx, child))
	if err != nil || exception != nil {
		t.Fatalf("child console: %v %v", err, exception)
	}
	filter := "child-console"
	for {
		logs, err := tab.Console().Read(ctx, browserproto.LogsInput{Tab: tab.ID(), Filter: &filter})
		if err != nil {
			t.Fatal(err)
		}
		if len(logs.Entries) == 1 {
			break
		}
		// A renderer round trip yields to the collector without a clock sleep.
		if _, _, err := runtime.Evaluate("0").Do(protocol.WithExecutor(ctx, child)); err != nil {
			t.Fatal(err)
		}
	}
	operation := tab.Operation(ctx, deadline())
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		t.Fatal("tab busy")
	}
	defer checkout.Release()
	_, err = tab.Navigate(
		ctx,
		&tabs.Visit{URL: server.URL + "/drop"},
		browserproto.LoadLoad,
		operation,
		&checkout.Session().References,
	)
	details := cdp.ErrorDetails(err)
	if cdp.ErrorCode(err) != "navigation_failed" || details.Action == nil || *details.Action != "completed" {
		t.Fatalf("navigation error=%v details=%+v", err, details)
	}
}

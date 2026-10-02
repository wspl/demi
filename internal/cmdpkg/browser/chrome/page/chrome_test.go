//go:build acceptance

package page_test

import (
	"bytes"
	"context"
	"encoding/json"
	"image/png"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// TestChromePage costs one pinned Chrome launch and a local fixture (budget 30 s).
// It observes actual DOM, native form state, screenshots and Host file transfers;
// scripted CDP replies cannot establish that Chrome applied these operations.
func TestChromePage(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to the pinned Chrome for Testing executable")
	}
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
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
<input id="file" type="file"><button id="submit" onclick="document.querySelector('#status').textContent=document.querySelector('#email').value+' '+document.querySelector('#date').value+' '+document.querySelector('#confirm').checked">Submit</button>
<p id="status">Ready</p><a id="download" href="/download">Download</a>
<svg width="20" height="20"><rect width="20" height="20" fill="red"/></svg>
</body></html>`))
	}))
	defer server.Close()
	environment := tabstest.Launch(ctx, t, tabs.LaunchOptions{Executable: executable, Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}})
	tab, err := environment.Open(ctx, server.URL, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(15 * time.Second)
	target := func(selector string) browserop.BrowserTarget {
		return browserop.BrowserTarget{BrowserQueryMatch: browserop.BrowserQueryMatch{CSS: &selector}}
	}
	tree, err := page.Inspect(ctx, tab, browserop.InspectInput{Tab: tab.ID()}, deadline)
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
	for _, fill := range []struct{ selector, text string }{{"#email", "agent@example.test"}, {"#date", "2026-10-03"}} {
		if _, err = page.Fill(ctx, tab, browserop.FillInput{Tab: tab.ID(), BrowserTarget: target(fill.selector), Text: fill.text}, deadline); err != nil {
			t.Fatal(err)
		}
	}
	if _, err = page.Check(ctx, tab, browserop.CheckInput{Tab: tab.ID(), BrowserTarget: target("#confirm"), Value: true}, deadline); err != nil {
		t.Fatal(err)
	}
	options := []string{"b"}
	if _, err = page.Select(ctx, tab, browserop.SelectInput{Tab: tab.ID(), BrowserTarget: target("#options"), Value: &options}, deadline); err != nil {
		t.Fatal(err)
	}
	if _, err = page.Click(ctx, tab, browserop.ClickInput{Tab: tab.ID(), BrowserTarget: target("#submit")}, deadline); err != nil {
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
	if _, err = page.Evaluate(ctx, tab, `document.querySelector('#status').textContent='changed'`, time.Second); err == nil {
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
	if err = os.WriteFile(file, []byte("upload fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	invocation := &cmdsdk.InvocationContext[commandwire.Invocation]{Request: commandwire.Invocation{Cwd: cwd}}
	uploaded, err := page.Upload(ctx, invocation, tab, browserop.UploadInput{Tab: tab.ID(), BrowserTarget: target("#file"), File: []browserop.LocatorText{browserop.LocatorText(file)}}, deadline)
	if err != nil || uploaded.Attached != 1 {
		t.Fatalf("upload=%+v err=%v", uploaded, err)
	}
	output := "download.txt"
	downloaded, err := page.Download(ctx, invocation, environment, tab, browserop.DownloadInput{Tab: tab.ID(), BrowserTarget: target("#download"), Output: &output}, deadline)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(downloaded.Path)
	if err != nil || string(data) != "downloaded fixture" {
		t.Fatalf("download=%q err=%v", data, err)
	}
	inventory, err := page.AssetsList(ctx, invocation, tab, browserop.AssetsListInput{Tab: tab.ID()}, deadline)
	if err != nil {
		t.Fatal(err)
	}
	if len(inventory.InlineSvgs) != 1 {
		t.Fatalf("SVG inventory=%+v", inventory)
	}
}

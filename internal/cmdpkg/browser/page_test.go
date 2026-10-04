//go:build acceptance

package browser

import (
	"bytes"
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page/pagetest"
)

func TestBrowserContractAndCleanup(t *testing.T) {
	f := chromeFixture(t)
	if len(f.tabs(t)) != 0 {
		t.Fatal("browser starts populated")
	}
	tab := f.open(t, "fixture.html")
	expectValue(t, f.eval(t, tab, "document.title"), "Native browser test")
	f.command(t, tab, "fill", `{"css":"#email","text":"hello@example.test"}`)
	f.click(t, tab, "#normal")
	expectValue(
		t,
		f.eval(t, tab, `[document.querySelector('#email').value,normalClicks]`),
		[]any{"hello@example.test", 1},
	)
	for _, css := range []string{"#covered", "#disabled", "#missing"} {
		code := "not_actionable"
		if css == "#missing" {
			code = "target_not_found"
		}
		f.rejects(t, tab, "click", browserArgs(t, `{"css":$0,"timeout":250}`, css), code, "")
	}
	err := pagetest.ClickCSS(t.Context(), f.tab(t, tab), ".duplicate", 5*time.Second)
	var ambiguous *cdp.BrowserError
	if !errors.As(err, &ambiguous) || ambiguous.Kind != cdp.KindAmbiguous || ambiguous.Count != 2 {
		t.Fatalf("duplicate target: %v", err)
	}
	expectValue(t, f.eval(t, tab, "[coveredClicks,overlayClicks,disabledClicks]"), []int{0, 0, 0})
	for _, expression := range []string{`document.body.setAttribute('data-mutated','yes')`, `localStorage.setItem('probe','yes')`, `fetch('/side-effect')`, `navigator.sendBeacon('/side-effect','x')`, `window.evil`, `setTimeout(()=>window.sideEffects++,0)`, `window.sideEffects++`, `({get x(){window.sideEffects++;return 1}})`, `({x:undefined})`, `({x:NaN})`, `document.body`, `(()=>{const a={};a.self=a;return a})()`, `[1,,3]`} {
		completion, _, stderr := f.result(t, "eval", browserArgs(t, `{"tab":$0,"expression":$1}`, tab, expression))
		if completion.ExitCode == 0 {
			t.Fatalf("accepted %s: %s", expression, stderr)
		}
	}
	expectValue(t, f.eval(t, tab, `(()=>{const a={x:1};return [a,a]})()`), []struct {
		X int `json:"x"`
	}{{1}, {1}})
	f.click(t, tab, "#check-storage")
	expectValue(
		t,
		f.eval(t, tab, `[document.body.hasAttribute('data-mutated'),window.storageValue,sideEffects]`),
		[]any{false, nil, 0},
	)
	expectValue(t, f.get(t, "/effects-count"), 0)
	f.click(t, tab, "#arm")
	f.click(t, tab, "#late")
	expectValue(t, f.eval(t, tab, "lateClicks"), 1)
	png, err := page.Screenshot(t.Context(), f.tab(t, tab), 5*time.Second)
	if err != nil || !bytes.HasPrefix(png, []byte("\x89PNG\r\n\x1a\n")) {
		t.Fatalf("screenshot is not PNG: %v", err)
	}
	other := f.open(t, "fixture.html")
	current := f.tab(t, tab)
	ctx, cancel := context.WithCancel(t.Context())
	var workers sync.WaitGroup
	pendingCtx, stopPending := context.WithCancel(t.Context())
	defer func() {
		cancel()
		stopPending()
		workers.Wait()
	}()
	waiting := make(chan error, 1)
	workers.Go(func() {
		waiting <- pagetest.ClickCSS(ctx, current, "#disabled", 5*time.Second)
	})
	f.waitBusy(t, tab)
	independent, err := page.Evaluate(t.Context(), f.tab(t, other), "1+1", 5*time.Second)
	cancel()
	canceled := <-waiting
	if err != nil {
		t.Fatal(err)
	}
	expectValue(t, independent, 2)
	if !errors.Is(canceled, context.Canceled) && cdp.ErrorCode(canceled) != "cancelled" {
		t.Fatalf("cancelled click: %v", canceled)
	}
	workers.Go(func() {
		waiting <- pagetest.ClickCSS(pendingCtx, current, "#disabled", 5*time.Second)
	})
	f.waitBusy(t, tab)
	_, busy := page.Evaluate(t.Context(), current, "1", 5*time.Second)
	result := <-waiting
	var failure *cdp.BrowserError
	if !errors.As(result, &failure) || failure.Kind != cdp.KindNotActionable {
		t.Fatalf("disabled click: %v", result)
	}
	if !errors.As(busy, &failure) || failure.Kind != cdp.KindBusy {
		t.Fatalf("competing evaluation: %v", busy)
	}
	retained := f.tab(t, other)
	f.command(t, tab, "close", `{}`)
	for _, row := range f.tabs(t) {
		if row.ID == tab {
			t.Fatal("closed tab listed")
		}
	}
	if err := f.s.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if _, err := page.Evaluate(t.Context(), retained, "1", 5*time.Second); cdp.ErrorCode(err) != "browser_lost" {
		t.Fatalf("retired page: %v", err)
	}
}

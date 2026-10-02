//go:build acceptance

package tabs_test

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

// Real Chrome is required to prove capture, renderer navigation and process/profile
// retirement together. The one environment is reused across the scenarios; the
// 30-second deadline guards hangs, while every wait observes a concrete event.
func TestChromeEnvironmentLifecycle(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("DEMI_TEST_CHROME supplies real Chrome for acceptance")
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if r.URL.Path == "/redirect" {
			http.Redirect(w, r, "/next", http.StatusFound)
			return
		}
		_, err := fmt.Fprintf(w, `<!doctype html><title>%s</title><style>body{background:#37a;color:white}</style><h1>Chrome tabs</h1><button id="popup" onclick="window.open('/popup')">Open popup</button><button id="dialog" onclick="alert('hello tabs')">Dialog</button><script>console.log('loaded %s');</script>`, r.URL.Path, r.URL.Path)
		if err != nil {
			t.Logf("page client left: %v", err)
		}
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
	environment := tabstest.Launch(ctx, t, tabs.LaunchOptions{Executable: executable, Locale: commandwire.CommandLocale{TimeZone: "Asia/Singapore", Languages: []commandwire.LanguageTag{"en-US", "zh-CN"}}})
	profile := filepath.Dir(environment.DownloadDirectory())
	first, err := environment.Open(ctx, server.URL+"/", 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	second, err := environment.Open(ctx, server.URL+"/next", 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("opened %s and %s", first.ID(), second.ID())

	t.Run("registry and locale", func(t *testing.T) {
		listed, err := environment.Listed(ctx, 5*time.Second)
		if err != nil {
			t.Fatal(err)
		}
		if len(listed.Tabs) != 2 || listed.Tabs[0].Tab != first || listed.Tabs[1].Tab != second {
			t.Fatalf("listing %+v", listed.Tabs)
		}
		if got := evaluateString(t, ctx, first, `JSON.stringify([navigator.webdriver,Intl.DateTimeFormat().resolvedOptions().timeZone,navigator.languages])`); got != `[false,"Asia/Singapore",["en-US","zh-CN"]]` {
			t.Fatal(got)
		}
		if strings.Contains(evaluateString(t, ctx, first, `navigator.userAgent`), "Headless") {
			t.Fatal("headless user agent")
		}
	})
	t.Run("navigation and references", func(t *testing.T) {
		checkout := first.Gate().TryCheckout()
		if checkout == nil {
			t.Fatal("tab busy")
		}
		defer checkout.Release()
		references := &checkout.Session().References
		old, err := references.Issue(tabs.Reference{Backend: 1, Frame: "old", Loader: "old"})
		if err != nil {
			t.Fatal(err)
		}
		operation := first.Operation(ctx, time.Now().Add(10*time.Second))
		defer operation.Close()
		final, err := first.Navigate(ctx, &tabs.Visit{URL: server.URL + "/redirect"}, browserop.LoadDomContentLoaded, operation, references)
		if err != nil {
			t.Fatal(err)
		}
		if final != server.URL+"/next" {
			t.Fatal(final)
		}
		if _, known := references.Lookup(old); known {
			t.Fatal("navigation retained old references")
		}
		if err := first.WaitCurrentLoad(ctx, browserop.LoadLoad); err != nil {
			t.Fatal(err)
		}
		history, err := first.HistoryStep(ctx, true, operation)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := first.Navigate(ctx, &tabs.History{EntryID: history.ID}, browserop.LoadDomContentLoaded, operation, references); err != nil {
			t.Fatal(err)
		}
		if evaluateString(t, ctx, first, `location.pathname`) != "/" {
			t.Fatal("history did not navigate")
		}
	})
	t.Run("popup registration", func(t *testing.T) {
		changed := environment.Latest().Changed
		evaluateString(t, ctx, first, `document.querySelector('#popup').click(); 'opened'`)
		for {
			ids, err := first.Popups(ctx)
			if err != nil {
				t.Fatal(err)
			}
			if len(ids) > 0 {
				popup, err := environment.Tab(ctx, ids[0], 5*time.Second)
				if err != nil {
					t.Fatal(err)
				}
				created, ok := popup.CreatedBy().(*browserop.BrowserCreatedByPage)
				if !ok || created.Opener != first.ID() {
					t.Fatal(popup.CreatedBy())
				}
				if err := popup.Close(ctx, 5*time.Second); err != nil {
					t.Fatal(err)
				}
				break
			}
			select {
			case <-changed:
				changed = environment.Latest().Changed
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
		}
	})
	t.Run("console and dialogs", func(t *testing.T) {
		// The subscription synchronizes command delivery; Read's cursor observes the
		// collector eventually, without a fixed delay or a wall-time assertion.
		events, err := first.Subscribe("Runtime.consoleAPICalled")
		if err != nil {
			t.Fatal(err)
		}
		defer events.Close()
		evaluateString(t, ctx, first, `console.log('tabs-console-marker'); 'logged'`)
		if _, err := events.Next(ctx); err != nil {
			t.Fatal(err)
		}
		for {
			logs, err := first.Console().Read(ctx, browserop.LogsInput{Tab: first.ID()})
			if err != nil {
				t.Fatal(err)
			}
			found := false
			for _, entry := range logs.Entries {
				found = found || strings.Contains(entry.Text, "tabs-console-marker")
			}
			if found {
				break
			}
			// A CDP round trip yields to the collector and bounds this event-based poll.
			evaluateString(t, ctx, second, `'collector barrier'`)
		}
		changedDialog := first.Dialog()
		finished := make(chan error, 1)
		go func() {
			_, _, err := runtime.Evaluate(`alert('hello tabs'); 'answered'`).Do(protocol.WithExecutor(ctx, first.Page()))
			finished <- err
		}()
		for {
			dialog, changed := changedDialog.Watch()
			if dialog != nil {
				if dialog.Message != "hello tabs" {
					t.Fatal(dialog.Message)
				}
				if err := changedDialog.Answer(ctx, dialog, true, nil); err != nil {
					t.Fatal(err)
				}
				if err := changedDialog.Answer(ctx, dialog, true, nil); cdp.ErrorCode(err) != browserop.BrowserErrorCode("dialog_not_found") {
					t.Fatalf("second answer: %v", err)
				}
				break
			}
			select {
			case <-changed:
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
		}
		if err := <-finished; err != nil {
			t.Fatal(err)
		}
	})
	t.Run("viewports and capture", func(t *testing.T) {
		viewport := browserop.BrowserViewport{Width: 700, Height: 500, DevicePixelRatio: 1, Mode: browserop.ViewportModeWeb}
		if err := first.SetViewport(ctx, viewport); err != nil {
			t.Fatal(err)
		}
		if err := first.UpdateScreen(ctx, tabs.Screen{Width: 1280, Height: 720, Ratio: 1}); err != nil {
			t.Fatal(err)
		}
		if got := evaluateString(t, ctx, first, `JSON.stringify([innerWidth,innerHeight])`); got != "[700,500]" {
			t.Fatal(got)
		}
		capture, err := environment.Captures().Start(ctx, first.TargetID(), 700, 500, 30, 2_000_000)
		if err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := capture.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}()
		// Repainting is explicit; a native idle page need not emit a frame.
		if err := tabs.Paint(ctx, first.Page()); err != nil {
			t.Fatal(err)
		}
		for {
			event, err := capture.Next(ctx)
			if err != nil {
				t.Fatal(err)
			}
			switch event := event.(type) {
			case *tabs.CaptureStarted:
			case *tabs.CaptureFrame:
				if len(event.Frame.Data) == 0 || event.Frame.Width != 700 || event.Frame.Height != 500 {
					t.Fatalf("bad capture frame %+v", event.Frame)
				}
				capture.Ack(event.Frame.Sequence, 2)
				t.Logf("captured %dx%d frame, %d bytes", event.Frame.Width, event.Frame.Height, len(event.Frame.Data))
				return
			case *tabs.CaptureStalled:
				t.Fatal("capture stalled")
			case *tabs.CaptureFailed:
				t.Fatal(event.Reason)
			}
		}
	})
	t.Run("temporary tabs hold retirement", func(t *testing.T) {
		batch, err := environment.TemporaryTabs(ctx, 7, 1, time.Now().Add(10*time.Second))
		if err != nil {
			t.Fatal(err)
		}
		if len(batch.Tabs()) != 1 {
			t.Fatal("missing temporary tab")
		}
		if err := batch.Close(ctx, 5*time.Second); err != nil {
			t.Fatal(err)
		}
	})
	if err := first.Close(ctx, 5*time.Second); err != nil {
		t.Fatal(err)
	}
	if err := second.Close(ctx, 5*time.Second); err != nil {
		t.Fatal(err)
	}
	select {
	case <-environment.Emptied():
	default:
		t.Fatal("last tab did not seal registry")
	}
	// The environment owner joins retirement before acknowledging final closure.
	if err := environment.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(profile); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("profile retained after final-tab closure: %v", err)
	}
	if err := environment.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	t.Log("last-tab closure reaped Chrome and removed its profile")
}

func evaluateString(t *testing.T, ctx context.Context, tab *tabs.Tab, expression string) string {
	t.Helper()
	value, exception, err := runtime.Evaluate(expression).WithReturnByValue(true).Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		t.Fatal(err)
	}
	if exception != nil {
		t.Fatal(exception)
	}
	result, err := contract.Decode[string](value.Value)
	if err != nil {
		t.Fatal(err)
	}
	return result
}

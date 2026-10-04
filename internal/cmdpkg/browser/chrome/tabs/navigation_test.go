package tabs

import (
	"context"
	"testing"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

func TestLiteralBrowserURLGlobs(t *testing.T) {
	for _, test := range []struct {
		pattern, url string
		match        bool
	}{
		{"/a/*", "/a/b/c", false},
		{"/a/**", "/a/b/c", true},
		{"/a/?", "/a/b", false},
		{"/a/?", "/a/?", true},
		{"/[a]{b}", "/[a]{b}", true},
		{"/[a]{b}", "/ab", false},
		{"https://example.test/文/*", "https://example.test/文/a", true},
	} {
		t.Run(test.pattern+test.url, func(t *testing.T) {
			matcher, err := urlPattern(test.pattern)
			if err != nil {
				t.Fatal(err)
			}
			if got := matcher.MatchString(test.url); got != test.match {
				t.Fatalf("matched %v, want %v", got, test.match)
			}
		})
	}
}

// A scripted acknowledgement distinguishes a failed load from undelivered input.
// No Chrome or clock waits; each case costs one local WebSocket connection.
func TestFailedNavigationPreservesAcknowledgedProgress(t *testing.T) {
	for _, test := range []struct {
		name     string
		result   any
		err      error
		progress browserop.ActionProgress
	}{
		{"network failure", page.NavigateReturns{FrameID: "frame", ErrorText: "net::ERR_EMPTY_RESPONSE"}, nil, "completed"},
		{"protocol refusal", nil, &cdp.ProtocolError{Code: -32000, Message: "navigation refused"}, "unknown"},
	} {
		t.Run(test.name, func(t *testing.T) {
			tree := page.GetFrameTreeReturns{
				FrameTree: &page.FrameTree{
					Frame: &protocol.Frame{
						ID:                             "frame",
						LoaderID:                       "old",
						URL:                            "about:blank",
						SecureContextType:              protocol.SecureContextTypeInsecureScheme,
						CrossOriginIsolatedContextType: protocol.CrossOriginIsolatedContextTypeNotIsolated,
					},
				},
			}
			server := cdptest.NewServer(
				t,
				cdptest.Exchange{
					Method: "Target.attachToTarget",
					Params: target.AttachToTarget("tab").WithFlatten(true),
					Result: target.AttachToTargetReturns{SessionID: "main"},
				},
				cdptest.Exchange{
					Method:    "Target.setAutoAttach",
					SessionID: "main",
					Params: target.SetAutoAttach(true, false).
						WithFlatten(true).
						WithFilter(target.Filter{
							{Type: "iframe"},
							{Type: "worker"},
							{Type: "shared_worker"},
							{Type: "service_worker"},
							{Exclude: true},
						}),
				},
				cdptest.Exchange{
					Method:    "Page.getFrameTree",
					SessionID: "main",
					Params:    page.GetFrameTree(),
					Result:    tree,
				},
				cdptest.Exchange{
					Method:    "Page.navigate",
					SessionID: "main",
					Params:    page.Navigate("http://example.test/drop"),
					Result:    test.result,
					Err:       test.err,
				},
				cdptest.Exchange{
					Method:    "Page.getFrameTree",
					SessionID: "main",
					Params:    page.GetFrameTree(),
					Result:    tree,
				},
			)
			connection, err := cdp.Dial(t.Context(), server.Address())
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				if err := connection.Close(context.Background()); err != nil {
					t.Error(err)
				}
			})
			session, err := connection.Attach(t.Context(), "tab")
			if err != nil {
				t.Fatal(err)
			}
			tab := &Tab{session: session, ctx: t.Context(), id: "t1"}
			operation := tab.Operation(t.Context(), time.Now().Add(time.Second))
			defer operation.Close()
			_, err = tab.Navigate(
				t.Context(),
				&Visit{URL: "http://example.test/drop"},
				browserop.LoadLoad,
				operation,
				&References{},
			)
			details := cdp.ErrorDetails(err)
			if err == nil || details.Action == nil || *details.Action != test.progress {
				t.Fatalf("navigation error=%v details=%+v, want action %s", err, details, test.progress)
			}
			if test.err == nil && cdp.ErrorCode(err) != "navigation_failed" {
				t.Fatal(err)
			}
		})
	}
}

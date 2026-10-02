package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// Navigation selects a URL, reload or history entry.
//
//sumtype:decl
type Navigation interface{ navigation() }

// Visit navigates to an absolute supported URL.
type Visit struct{ URL string }

func (*Visit) navigation() {}

// Reload loads the current document again.
type Reload struct{}

func (*Reload) navigation() {}

// History navigates to a Chrome history entry ID.
type History struct{ EntryID int64 }

func (*History) navigation() {}

// NavigationObservation owns subscriptions installed before an action starts.
// A caller must Close it on success, failure and cancellation.
type NavigationObservation struct{}

// DocumentChanged reports an observed cross-document commit.
func (n *NavigationObservation) DocumentChanged() bool { panic("not written: k-chrome-tabs") }

// URL returns the most recently observed main-frame URL.
func (n *NavigationObservation) URL() string { panic("not written: k-chrome-tabs") }

// WaitLoad observes the requested lifecycle under the operation's shared deadline.
// Ordinary clicks use Rust's 250 ms load-start classification window only.
func (n *NavigationObservation) WaitLoad(ctx context.Context, load browserop.Load, ordinaryClick bool, operation *cdp.Operation) error {
	panic("not written: k-chrome-tabs")
}

// WaitURL observes navigation until the main URL matches the browser URL glob.
func (n *NavigationObservation) WaitURL(ctx context.Context, executor cdp.Executor, pattern string) error {
	panic("not written: k-chrome-tabs")
}

// Close releases every navigation subscription.
func (n *NavigationObservation) Close() { panic("not written: k-chrome-tabs") }

// ObserveNavigation installs an observer on the tab's session before page input.
func (t *Tab) ObserveNavigation(ctx context.Context) (*NavigationObservation, error) {
	panic("not written: k-chrome-tabs")
}

// Navigate runs one navigation, invalidates document references and returns its URL.
// The caller holds the tab gate and passes that checkout's references.
func (t *Tab) Navigate(ctx context.Context, navigation Navigation, load browserop.Load, operation *cdp.Operation, references *References) (string, error) {
	panic("not written: k-chrome-tabs")
}

// WaitCurrentLoad waits for this tab's current document load state.
func (t *Tab) WaitCurrentLoad(ctx context.Context, load browserop.Load) error {
	panic("not written: k-chrome-tabs")
}

// HistoryStep selects the next back or forward entry without navigating.
func (t *Tab) HistoryStep(ctx context.Context, back bool, operation *cdp.Operation) (*page.NavigationEntry, error) {
	panic("not written: k-chrome-tabs")
}

// Reload starts an owned background reload bounded to 60 seconds. Tab closure
// cancels and joins it; the live view displays Chrome's load or error page.
func (t *Tab) Reload() { panic("not written: k-chrome-tabs") }

// Steer starts a user's goto, reload, back or forward without waiting for load.
func (t *Tab) Steer(ctx context.Context, command browserop.Operation, operation *cdp.Operation) (browserop.NavigationResult, error) {
	panic("not written: k-chrome-tabs")
}

// ValidateURL rejects URLs outside the browser navigation contract.
func ValidateURL(url string) error { panic("not written: k-chrome-tabs") }

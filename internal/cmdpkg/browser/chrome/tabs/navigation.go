package tabs

import (
	"context"
	"errors"
	"regexp"
	"strings"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/cdproto/runtime"
	urlparser "github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/contract"

	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// Navigation selects a URL, reload or history entry.
//
//sumtype:decl
type Navigation interface{ navigation() }

// Visit navigates to an absolute supported URL.
type Visit struct {
	// URL names the navigation destination.
	URL string
}

func (*Visit) navigation() {}

// Reload loads the current document again.
type Reload struct{}

func (*Reload) navigation() {}

// History navigates to a Chrome history entry ID.
type History struct {
	// EntryID selects the Chrome history entry.
	EntryID int64
}

func (*History) navigation() {}

// NavigationObservation owns subscriptions installed before an action starts.
// A caller must Close it on success, failure and cancellation.
type NavigationObservation struct {
	events    *cdp.Subscription
	frame     protocol.FrameID
	previous  protocol.LoaderID
	expected  protocol.LoaderID
	committed protocol.LoaderID
	documents map[network.RequestID]protocol.LoaderID
	states    map[protocol.LoaderID]map[string]bool
	url       string
}

// DocumentChanged reports an observed cross-document commit.
func (n *NavigationObservation) DocumentChanged() bool {
	return n.committed != ""
}

// URL returns the most recently observed main-frame URL.
func (n *NavigationObservation) URL() string {
	return n.url
}

// WaitLoad observes the requested lifecycle under the operation's shared deadline.
// An ordinary click that starts no load within 250 ms counts as not navigating.
func (n *NavigationObservation) WaitLoad(
	ctx context.Context,
	load browserop.Load,
	ordinaryClick bool,
	operation *cdp.Operation,
) error {
	return operation.Run(ctx, func(ctx context.Context) error {
		return n.waitLoad(ctx, load, ordinaryClick, operation)
	})
}

// WaitURL observes navigation until the main URL matches the browser URL glob.
func (n *NavigationObservation) WaitURL(ctx context.Context, executor cdp.Executor, pattern string) error {
	matcher, err := urlPattern(pattern)
	if err != nil {
		return err
	}
	drain, cancel := context.WithCancel(ctx)
	cancel()
	for {
		event, _, err := n.next(drain)
		if errors.Is(err, context.Canceled) {
			break
		}
		if err != nil {
			return err
		}
		if (event == navigationSame || event == navigationCommit) && matcher.MatchString(n.url) {
			return nil
		}
	}
	tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, executor))
	if err != nil {
		return err
	}
	n.url = frameURL(tree.Frame)
	if matcher.MatchString(n.url) {
		return nil
	}
	for {
		event, _, err := n.next(ctx)
		if err != nil {
			return err
		}
		if (event == navigationSame || event == navigationCommit) && matcher.MatchString(n.url) {
			return nil
		}
	}
}

// Close releases every navigation subscription.
func (n *NavigationObservation) Close() {
	n.events.Close()
}

// ObserveNavigation installs an observer on the tab's session before page input.
func (t *Tab) ObserveNavigation(ctx context.Context) (*NavigationObservation, error) {
	events, err := t.session.SubscribeWithCapacity(
		256,
		"Network.requestWillBeSent",
		"Network.loadingFailed",
		"Page.frameNavigated",
		"Page.navigatedWithinDocument",
		"Page.lifecycleEvent",
		"Page.frameStartedLoading",
	)
	if err != nil {
		return nil, err
	}
	tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, t.Page()))
	if err != nil {
		events.Close()
		return nil, err
	}
	n := &NavigationObservation{
		events:    events,
		frame:     tree.Frame.ID,
		previous:  tree.Frame.LoaderID,
		url:       frameURL(tree.Frame),
		documents: make(map[network.RequestID]protocol.LoaderID),
		states:    make(map[protocol.LoaderID]map[string]bool),
	}
	drain, cancel := context.WithCancel(ctx)
	cancel()
	for {
		_, _, err := n.next(drain)
		if errors.Is(err, context.Canceled) {
			break
		}
		if err != nil {
			events.Close()
			return nil, err
		}
	}
	n.expected = ""
	n.committed = ""
	clear(n.documents)
	clear(n.states)
	return n, nil
}

// Navigate runs one navigation, invalidates document references and returns its URL.
// The caller holds the tab gate and passes that checkout's references.
func (t *Tab) Navigate(
	ctx context.Context,
	navigation Navigation,
	load browserop.Load,
	operation *cdp.Operation,
	references *References,
) (string, error) {
	if visit, ok := navigation.(*Visit); ok {
		if err := ValidateURL(visit.URL); err != nil {
			return "", err
		}
	}
	var observation *NavigationObservation
	err := operation.Run(ctx, func(ctx context.Context) error {
		var err error
		observation, err = t.ObserveNavigation(ctx)
		return err
	})
	if err != nil {
		return "", err
	}
	defer observation.Close()
	err = operation.Run(ctx, func(ctx context.Context) error {
		operation.BeginInput()
		if err := dispatchNavigation(ctx, t.Page(), navigation); err != nil {
			var failure *cdp.BrowserError
			if errors.As(err, &failure) && failure.Kind == cdp.KindNavigationFailed {
				operation.CompleteInput()
			}
			return err
		}
		operation.CompleteInput()
		return observation.WaitLoad(ctx, load, false, operation)
	})
	if observation.DocumentChanged() {
		references.Invalidate()
	}
	if err != nil {
		diagnostic, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		defer cancel()
		tree, diagnosticErr := page.GetFrameTree().Do(protocol.WithExecutor(diagnostic, t.Page()))
		if diagnosticErr == nil {
			observation.url = frameURL(tree.Frame)
		}
		return "", operation.Failure(err, string(t.id), &observation.url)
	}
	return observation.url, nil
}

// WaitCurrentLoad waits for this tab's current document load state.
func (t *Tab) WaitCurrentLoad(ctx context.Context, load browserop.Load) error {
	events, err := t.session.SubscribeWithCapacity(256, "Page.lifecycleEvent", "Page.frameNavigated")
	if err != nil {
		return err
	}
	defer events.Close()
	renderer := protocol.WithExecutor(ctx, t.Page())
	tree, err := page.GetFrameTree().Do(renderer)
	if err != nil {
		return err
	}
	frame := tree.Frame
	result, exception, err := runtime.Evaluate("document.readyState").WithReturnByValue(true).Do(renderer)
	if err != nil {
		return err
	}
	if exception != nil {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Cause: exception, Message: exception.Error()}
	}
	ready, err := contract.Decode[string](result.Value)
	if err != nil {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Cause: err, Message: err.Error()}
	}
	tree, err = page.GetFrameTree().Do(renderer)
	if err != nil {
		return err
	}
	replaced := func() error {
		return &cdp.BrowserError{
			Kind:    cdp.KindNavigationFailed,
			Message: "current document was replaced before its load wait completed",
		}
	}
	if tree.Frame.LoaderID != frame.LoaderID {
		return replaced()
	}
	if load == browserop.LoadCommit || ready == "complete" ||
		load == browserop.LoadDOMContentLoaded && ready == "interactive" {
		return nil
	}
	for {
		raw, err := events.Next(ctx)
		if err != nil {
			return err
		}
		decoded, err := cdp.DecodeEvent(raw)
		if err != nil {
			return err
		}
		switch event := decoded.(type) {
		case *page.EventFrameNavigated:
			if event.Frame.ID == frame.ID && event.Frame.LoaderID != frame.LoaderID {
				return replaced()
			}
		case *page.EventLifecycleEvent:
			if event.FrameID == frame.ID && event.LoaderID == frame.LoaderID &&
				(event.Name == "load" || event.Name == loadEvent(load)) {
				return nil
			}
		}
	}
}

// HistoryStep selects the next back or forward entry without navigating.
func (t *Tab) HistoryStep(ctx context.Context, back bool, operation *cdp.Operation) (*page.NavigationEntry, error) {
	var entry *page.NavigationEntry
	err := operation.Run(ctx, func(ctx context.Context) error {
		index, entries, err := page.GetNavigationHistory().Do(protocol.WithExecutor(ctx, t.Page()))
		if err != nil {
			return err
		}
		if back {
			index--
		} else {
			index++
		}
		if index < 0 || index >= int64(len(entries)) {
			return &cdp.BrowserError{Kind: cdp.KindHistoryBoundary}
		}
		entry = entries[index]
		return nil
	})
	return entry, err
}

// Reload starts an owned background reload bounded to 60 seconds. Tab closure
// cancels and joins it; the live view displays Chrome's load or error page.
func (t *Tab) Reload() {
	t.detach(&Reload{})
}

// Steer starts a user's goto, reload, back or forward without waiting for load.
func (t *Tab) Steer(
	ctx context.Context,
	command browserop.Operation,
	operation *cdp.Operation,
) (browserop.NavigationResult, error) {
	var url string
	switch command := command.(type) {
	case *browserop.GotoInput:
		if err := ValidateURL(command.URL); err != nil {
			return browserop.NavigationResult{}, err
		}
		url = command.URL
		t.detach(&Visit{URL: url})
	case *browserop.ReloadInput:
		err := operation.Run(ctx, func(ctx context.Context) error {
			tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, t.Page()))
			if err == nil {
				url = frameURL(tree.Frame)
			}
			return err
		})
		if err != nil {
			return browserop.NavigationResult{}, err
		}
		if url == "" {
			return browserop.NavigationResult{}, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "the tab has no URL to reload",
			}
		}
		t.Reload()
	case *browserop.BackInput, *browserop.ForwardInput:
		_, back := command.(*browserop.BackInput)
		entry, err := t.HistoryStep(ctx, back, operation)
		if err != nil {
			return browserop.NavigationResult{}, err
		}
		url = entry.URL
		t.detach(&History{EntryID: entry.ID})
	default:
		return browserop.NavigationResult{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "not a navigation command",
		}
	}
	return browserop.NavigationResult{Tab: t.id, URL: url}, nil
}

// ValidateURL rejects URLs outside the browser navigation contract.
func ValidateURL(url string) error {
	parsed, err := urlparser.Parse(url)
	if err != nil {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: err.Error(), Cause: err}
	}
	switch parsed.Scheme() {
	case "http", "https", "file":
		return nil
	}
	if parsed.String() == "about:blank" {
		return nil
	}
	return &cdp.BrowserError{
		Kind:    cdp.KindConfiguration,
		Message: "navigation accepts http:, https:, file:, or about:blank",
	}
}

type navigationEvent uint8

const (
	navigationOther navigationEvent = iota
	navigationStarted
	navigationSame
	navigationCommit
	navigationLifecycle
	navigationFailed
)

// next applies only main-document events, retaining requested-loader identity.
func (n *NavigationObservation) next(ctx context.Context) (navigationEvent, string, error) {
	raw, err := n.events.Next(ctx)
	if err != nil {
		return navigationOther, "", err
	}
	decoded, err := cdp.DecodeEvent(raw)
	if err != nil {
		return navigationOther, "", err
	}
	switch event := decoded.(type) {
	case *network.EventRequestWillBeSent:
		if event.FrameID == n.frame && event.Type == network.ResourceTypeDocument {
			n.documents[event.RequestID] = event.LoaderID
			if event.LoaderID != n.previous && n.expected == "" {
				n.expected = event.LoaderID
			}
			return navigationStarted, "", nil
		}
	case *network.EventLoadingFailed:
		if loader, ok := n.documents[event.RequestID]; ok && (n.expected == "" || n.expected == loader) {
			return navigationFailed, event.ErrorText, nil
		}
	case *page.EventNavigatedWithinDocument:
		if event.FrameID == n.frame {
			n.url = event.URL
			return navigationSame, "", nil
		}
	case *page.EventFrameNavigated:
		return n.committedFrame(event)
	case *page.EventLifecycleEvent:
		if event.FrameID == n.frame {
			states := n.states[event.LoaderID]
			if states == nil {
				states = make(map[string]bool)
				n.states[event.LoaderID] = states
			}
			states[event.Name] = true
			return navigationLifecycle, "", nil
		}
	case *page.EventFrameStartedLoading:
		if event.FrameID == n.frame {
			return navigationStarted, "", nil
		}
	}
	return navigationOther, "", nil
}

func frameURL(frame *protocol.Frame) string { return frame.URL + frame.URLFragment }
func loadEvent(load browserop.Load) string {
	if load == browserop.LoadLoad {
		return "load"
	}
	return "DOMContentLoaded"
}

// urlPattern compiles browser URL globs, with every non-star character literal.
func urlPattern(pattern string) (*regexp.Regexp, error) {
	var expression strings.Builder
	expression.WriteString(`\A`)
	for index := 0; index < len(pattern); index++ {
		if pattern[index] == '*' {
			if index+1 < len(pattern) && pattern[index+1] == '*' {
				index++
				expression.WriteString("(?s:.*)")
			} else {
				expression.WriteString("[^/]*")
			}
		} else {
			expression.WriteString(regexp.QuoteMeta(pattern[index : index+1]))
		}
	}
	expression.WriteString(`\z`)
	matcher, err := regexp.Compile(expression.String())
	if err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Cause: err, Message: err.Error()}
	}
	return matcher, nil
}

// dispatchNavigation sends one native navigation without replaying its side effects.
func dispatchNavigation(ctx context.Context, executor cdp.Executor, navigation Navigation) error {
	renderer := protocol.WithExecutor(ctx, executor)
	switch navigation := navigation.(type) {
	case *Visit:
		_, _, reason, _, err := page.Navigate(navigation.URL).Do(renderer)
		if err != nil {
			return err
		}
		if reason != "" {
			return &cdp.BrowserError{Kind: cdp.KindNavigationFailed, Message: reason}
		}
		return nil
	case *Reload:
		return page.Reload().Do(renderer)
	case *History:
		return page.NavigateToHistoryEntry(navigation.EntryID).Do(renderer)
	}
	return nil
}

// detach registers address-bar navigation in the tab lifetime, bounded to 60 seconds.
func (t *Tab) detach(navigation Navigation) {
	if err := t.StartTask(func(ctx context.Context) {
		bounded, cancel := context.WithTimeout(ctx, 60*time.Second)
		defer cancel()
		// The work panel shows Chrome's own loading/error page; no invocation awaits it.
		_ = dispatchNavigation(bounded, t.Page(), navigation)
	}); err != nil {
		return
	} // A closing tab must not receive a late navigation.
}

func (n *NavigationObservation) waitLoad(
	ctx context.Context,
	load browserop.Load,
	ordinaryClick bool,
	operation *cdp.Operation,
) error {
	classification, cancel := context.WithTimeout(ctx, 250*time.Millisecond)
	defer cancel()
	started := !ordinaryClick
	for {
		waitCtx := ctx
		if !started {
			waitCtx = classification
		}
		event, reason, err := n.next(waitCtx)
		if err != nil {
			if !started && errors.Is(err, context.DeadlineExceeded) && ctx.Err() == nil {
				return nil
			}
			return err
		}
		if event == navigationStarted || event == navigationCommit || event == navigationSame {
			operation.CompleteInput()
		}
		switch event {
		case navigationSame:
			if n.expected == "" {
				return nil
			}
		case navigationFailed:
			if ordinaryClick {
				return nil
			}
			return &cdp.BrowserError{Kind: cdp.KindNavigationFailed, Message: reason}
		case navigationStarted, navigationCommit:
			started = true
		}
		if n.committed != "" && (load == browserop.LoadCommit || n.states[n.committed][loadEvent(load)]) {
			return nil
		}
	}
}

func (n *NavigationObservation) committedFrame(event *page.EventFrameNavigated) (navigationEvent, string, error) {
	if event.Frame.ID == n.frame {
		n.url = frameURL(event.Frame)
		if (n.expected == "" || n.expected == event.Frame.LoaderID) && event.Frame.LoaderID != n.previous {
			n.committed = event.Frame.LoaderID
			if event.Type == page.NavigationTypeBackForwardCacheRestore {
				n.states[n.committed] = map[string]bool{"DOMContentLoaded": true, "load": true}
			}
		}
		return navigationCommit, "", nil
	}

	return navigationOther, "", nil
}

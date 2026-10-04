package live

import (
	"bytes"
	"context"
	_ "embed"
	"errors"
	"log/slog"
	"sync"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/emulation"
	chromepage "github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/contract"
)

const (
	observerWorld   = "demi-live"
	observerBinding = "demiLiveReport"
)

//go:embed observer.js
var observerSource string

type observedState struct {
	controls                       []browserproto.LiveControl
	cursor                         string
	editable                       bool
	documents                      uint64
	controlsVersion, cursorVersion uint64
	changed                        chan struct{}
}
type observed struct {
	// mu protects immutable publications and bounded copy subscriptions.
	mu     sync.Mutex
	state  observedState
	copies map[chan string]struct{}
}
type observerEntry struct {
	ready    chan struct{}
	observed *observed
	err      error
}

func (h *Hub) observer(ctx context.Context, tab *tabs.Tab) (*observed, error) {
	h.mu.Lock()
	for other := range h.observers {
		if other.Context().Err() != nil {
			delete(h.observers, other)
		}
	}
	entry := h.observers[tab]
	fresh := entry == nil
	if fresh {
		entry = &observerEntry{ready: make(chan struct{})}
		h.observers[tab] = entry
	}
	h.mu.Unlock()
	if fresh {
		entry.observed, entry.err = startObserver(ctx, tab)
		close(entry.ready)
		if entry.err != nil {
			h.mu.Lock()
			if h.observers[tab] == entry {
				delete(h.observers, tab)
			}
			h.mu.Unlock()
		}
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-entry.ready:
		return entry.observed, entry.err
	}
}

func (o *observed) snapshot() observedState {
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.state
}

func (o *observed) subscribe() (chan string, func()) {
	copies := make(chan string, 4)
	o.mu.Lock()
	o.copies[copies] = struct{}{}
	o.mu.Unlock()
	return copies, func() {
		o.mu.Lock()
		delete(o.copies, copies)
		o.mu.Unlock()
	}
}

func (o *observed) publish(change func(*observedState)) {
	o.mu.Lock()
	previous := o.state.changed
	change(&o.state)
	o.state.changed = make(chan struct{})
	o.mu.Unlock()
	close(previous)
}

func startObserver(ctx context.Context, tab *tabs.Tab) (*observed, error) {
	events, err := tab.Subscribe("Runtime.bindingCalled", "Page.frameNavigated")
	if err != nil {
		return nil, err
	}
	admitted := false
	defer func() {
		if !admitted {
			events.Close()
		}
	}()
	execution := protocol.WithExecutor(ctx, tab.Page())
	if err := runtime.AddBinding(observerBinding).WithExecutionContextName(observerWorld).Do(execution); err != nil {
		return nil, err
	}
	if _, err := chromepage.AddScriptToEvaluateOnNewDocument(observerSource).
		WithWorldName(observerWorld).
		WithRunImmediately(true).
		Do(execution); err != nil {
		return nil, err
	}
	if err := emulation.SetFocusEmulationEnabled(true).Do(execution); err != nil {
		return nil, err
	}
	if page.ClipboardUnisolated() == nil {
		if err := page.GrantClipboard(ctx, tab.Browser()); err != nil {
			return nil, err
		}
	}
	o := &observed{
		state:  observedState{controls: []browserproto.LiveControl{}, cursor: "default", changed: make(chan struct{})},
		copies: make(map[chan string]struct{}),
	}
	err = tab.StartTask(func(owner context.Context) {
		o.runObserver(owner, events)
	})
	if err != nil {
		return nil, err
	}
	admitted = true
	return o, nil
}

func observerCall(ctx context.Context, tab *tabs.Tab, expression string, byValue bool) (*runtime.RemoteObject, error) {
	execution := protocol.WithExecutor(ctx, tab.Page())
	tree, err := chromepage.GetFrameTree().Do(execution)
	if err != nil {
		return nil, err
	}
	world, err := chromepage.CreateIsolatedWorld(tree.Frame.ID).WithWorldName(observerWorld).Do(execution)
	if err != nil {
		return nil, err
	}
	result, exception, err := runtime.Evaluate(observerSource + "\n" + expression).
		WithContextID(world).
		WithAwaitPromise(true).
		WithReturnByValue(byValue).
		Do(execution)
	if err != nil {
		return nil, err
	}
	if exception != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: exception.Text}
	}
	return result, nil
}

func choose(
	ctx context.Context,
	tab *tabs.Tab,
	token browserproto.ControlToken,
	revision uint64,
	value string,
	indices []uint32,
) (bool, error) {
	message, err := contract.EncodeJSON(struct {
		Token    browserproto.ControlToken `json:"token"`
		Revision uint64                    `json:"revision"`
		Value    string                    `json:"value"`
		Indices  []uint32                  `json:"indices"`
	}{token, revision, value, indices})
	if err != nil {
		return false, err
	}
	result, err := observerCall(ctx, tab, "globalThis.demiLive.commit("+string(message)+")", true)
	return err == nil && bytes.Equal(bytes.TrimSpace(result.Value), []byte("true")), err
}

func attach(
	ctx context.Context,
	tab *tabs.Tab,
	token browserproto.ControlToken,
	revision uint64,
	files []string,
) (bool, error) {
	message, err := contract.EncodeJSON(struct {
		Token    browserproto.ControlToken `json:"token"`
		Revision uint64                    `json:"revision"`
	}{token, revision})
	if err != nil {
		return false, err
	}
	element, err := observerCall(ctx, tab, "globalThis.demiLive.element("+string(message)+")", false)
	if err != nil {
		return false, err
	}
	if element.ObjectID == "" {
		return false, nil
	}
	defer func() {
		cleanup := protocol.WithExecutor(context.WithoutCancel(ctx), tab.Page())
		if err := runtime.ReleaseObject(element.ObjectID).Do(cleanup); err != nil {
			slog.Debug("live observer object release", "error", err)
		}
	}()
	execution := protocol.WithExecutor(ctx, tab.Page())
	node, err := dom.DescribeNode().WithObjectID(element.ObjectID).Do(execution)
	if err != nil {
		return false, err
	}
	if node.LocalName != "input" {
		return false, nil
	}
	if err := dom.SetFileInputFiles(files).WithBackendNodeID(node.BackendNodeID).Do(execution); err != nil {
		return false, err
	}
	_, err = observerCall(ctx, tab, "globalThis.demiLive.committed("+string(message)+")", true)
	return err == nil, err
}

func writeClipboard(ctx context.Context, tab *tabs.Tab, text, html string) (bool, error) {
	capability, err := page.ClipboardCapability(ctx, tab)
	if err != nil || !capability.Available {
		return false, err
	}
	if err := page.GrantClipboard(ctx, tab.Browser()); err != nil {
		return false, err
	}
	data, err := contract.EncodeJSON(struct {
		Text string `json:"text"`
		HTML string `json:"html"`
	}{text, html})
	if err != nil {
		return false, err
	}
	result, err := observerCall(ctx, tab, `(async ({ text, html }) => {
 const items = { 'text/plain': new Blob([text], { type: 'text/plain' }) };
 if (html) items['text/html'] = new Blob([html], { type: 'text/html' });
 try { await navigator.clipboard.write([new ClipboardItem(items)]); return true; }
 catch { return false; }
 })(`+string(data)+`)`, true)
	return err == nil && bytes.Equal(bytes.TrimSpace(result.Value), []byte("true")), err
}

func (o *observed) runObserver(owner context.Context, events *cdp.Subscription) {
	defer events.Close()
	for {
		event, err := events.Next(owner)
		if err != nil {
			if errors.Is(err, cdp.ErrEventsLost) {
				continue
			}
			return
		}
		decoded, err := cdp.DecodeEvent(event)
		if err != nil {
			slog.Warn("live view observer event", "error", err)
			continue
		}
		switch value := decoded.(type) {
		case *runtime.EventBindingCalled:
			if value.Name != observerBinding {
				continue
			}
			report, err := decodeObserverReport([]byte(value.Payload))
			if err != nil {
				slog.Warn("live view observer report", "error", err)
				continue
			}
			o.publishReport(report)
		case *chromepage.EventFrameNavigated:
			if value.Frame.ParentID == "" {
				o.publish(func(s *observedState) {
					s.controls = []browserproto.LiveControl{}
					s.cursor = "default"
					s.editable = false
					s.documents++
					s.cursorVersion++
					s.controlsVersion++
				})
			}
		}
	}
}

func (o *observed) publishReport(report observerReport) {
	switch report := report.(type) {
	case *cursorReport:
		o.publish(func(s *observedState) {
			s.cursor, s.editable = report.Cursor, report.Editable
			s.cursorVersion++
		})
	case *controlsReport:
		o.publish(func(s *observedState) {
			s.controls = report.Controls
			s.controlsVersion++
		})
	case *copyReport:
		o.mu.Lock()
		subscribers := make([]chan string, 0, len(o.copies))
		for copies := range o.copies {
			subscribers = append(subscribers, copies)
		}
		o.mu.Unlock()
		for _, copies := range subscribers {
			// Keep the newest four copies for each subscriber: a full channel drops its oldest copy.
			select {
			case copies <- report.Text:
			default:
				select {
				case <-copies:
				default:
				}
				select {
				case copies <- report.Text:
				default:
				}
			}
		}
	}
}

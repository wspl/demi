package tabs

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"

	"github.com/chromedp/cdproto/input"
	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
)

// InputRelease is a key or mouse release held back while a dialog blocks input.
//
//sumtype:decl
type InputRelease interface{ inputRelease() }

// MouseRelease carries a native mouse button release.
type MouseRelease struct {
	// Event holds the deferred mouse release.
	Event *input.DispatchMouseEventParams
}

func (*MouseRelease) inputRelease() {}

// KeyRelease carries a native key release.
type KeyRelease struct {
	// Event holds the deferred key release.
	Event *input.DispatchKeyEventParams
}

func (*KeyRelease) inputRelease() {}

// DialogInput is access to the tab-owned dialog and deferred-release worker.
// The tab starts, cancels and joins this worker; consumers do not close it.
type DialogInput struct {
	tab      *Tab
	requests chan dialogRequest
	// mu protects the immutable current dialog and its notification only.
	mu      sync.Mutex
	dialog  *page.EventJavascriptDialogOpening
	changed chan struct{}
}

// Open returns the immutable open dialog, or nil. Preserve its pointer for Answer.
func (d *DialogInput) Open() *page.EventJavascriptDialogOpening {
	dialog, _ := d.Watch()
	return dialog
}

// IsOpen reports whether a dialog currently blocks native input.
func (d *DialogInput) IsOpen() bool {
	return d.Open() != nil
}

// Watch returns the current dialog and notification from one publication.
func (d *DialogInput) Watch() (*page.EventJavascriptDialogOpening, <-chan struct{}) {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.dialog, d.changed
}

// Defer hands releases to the tab owner in order. A full queue waits with ctx;
// after successful admission, the owner retains them beyond caller cancellation.
func (d *DialogInput) Defer(ctx context.Context, releases []InputRelease) error {
	select {
	case d.requests <- dialogRequest{releases: releases}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	case <-d.tab.ctx.Done():
		return nil // Input no longer matters after its tab closes.
	}
}

// Answer answers this exact observed dialog unless someone already answered it,
// then delivers deferred releases. A later dialog is never cleared by this answer.
func (d *DialogInput) Answer(
	ctx context.Context,
	dialog *page.EventJavascriptDialogOpening,
	accept bool,
	text *string,
) error {
	reply := make(chan error, 1)
	select {
	case d.requests <- dialogRequest{dialog: dialog, accept: accept, text: text, reply: reply}:
	case <-ctx.Done():
		return ctx.Err()
	case <-d.tab.ctx.Done():
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	select {
	case err := <-reply:
		return err
	case <-ctx.Done():
		return ctx.Err()
	case <-d.tab.ctx.Done():
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	}
}

// DialogType converts Chrome's dialog type to the browser result and live-view type.
func DialogType(kind page.DialogType) browserproto.DialogType {
	switch kind {
	case page.DialogTypeAlert:
		return browserproto.DialogTypeAlert
	case page.DialogTypeConfirm:
		return browserproto.DialogTypeConfirm
	case page.DialogTypePrompt:
		return browserproto.DialogTypePrompt
	case page.DialogTypeBeforeunload:
		return browserproto.DialogTypeBeforeUnload
	}
	return browserproto.DialogType(kind)
}

type dialogRequest struct {
	releases []InputRelease
	dialog   *page.EventJavascriptDialogOpening
	accept   bool
	text     *string
	reply    chan error
}
type dialogOpening struct {
	dialog *page.EventJavascriptDialogOpening
	err    error
}

// observeDialogs starts the one owner of a page's dialogs and held input releases.
func observeDialogs(tab *Tab) (*DialogInput, error) {
	subscription, err := tab.Subscribe("Page.javascriptDialogOpening")
	if err != nil {
		return nil, err
	}
	d := &DialogInput{tab: tab, requests: make(chan dialogRequest, 16), changed: make(chan struct{})}
	err = tab.StartTask(func(ctx context.Context) {
		defer subscription.Close()
		opening := make(chan dialogOpening)
		stopped := make(chan struct{})
		readerCtx, cancel := context.WithCancel(ctx)
		defer func() {
			cancel()
			<-stopped
		}()
		go func() {
			defer close(stopped)
			for {
				raw, err := subscription.Next(readerCtx)
				if readerCtx.Err() != nil {
					return
				}
				item := dialogOpening{err: err}
				if err == nil {
					decoded, err := cdp.DecodeEvent(raw)
					item.err = err
					if event, ok := decoded.(*page.EventJavascriptDialogOpening); ok {
						item.dialog = event
					}
				}
				select {
				case opening <- item:
				case <-readerCtx.Done():
					return
				}
				if item.err != nil {
					return
				}
			}
		}()
		d.run(ctx, opening)
	})
	if err != nil {
		subscription.Close()
		return nil, err
	}
	return d, nil
}

func (d *DialogInput) publish(dialog *page.EventJavascriptDialogOpening) {
	d.mu.Lock()
	old := d.changed
	d.dialog = dialog
	d.changed = make(chan struct{})
	d.mu.Unlock()
	close(old)
}

// opened fails the tab when loss means its dialog state can no longer be known.
func (d *DialogInput) opened(event dialogOpening) error {
	if event.err != nil {
		failure := &cdp.BrowserError{
			Kind:    cdp.KindConnection,
			Message: fmt.Sprintf("browser dialog observation failed: %v", event.err),
			Cause:   event.err,
		}
		d.tab.cancel(failure)
		return failure
	}
	d.publish(event.dialog)
	return nil
}

func (d *DialogInput) run(ctx context.Context, opening <-chan dialogOpening) {
	var releases []InputRelease
	for {
		select {
		case <-ctx.Done():
			return
		case event := <-opening:
			if d.opened(event) != nil {
				return
			}
		case request := <-d.requests:
			releases = append(releases, request.releases...)
			var err error
			if request.reply != nil {
				err = d.answer(ctx, request)
			}
			if err == nil {
				releases, err = d.deliver(ctx, releases, opening)
			}
			if request.reply != nil {
				request.reply <- err
			}
			// Deferred release failures are only logged: no caller waits for them.
			if err != nil && request.reply == nil && !errors.Is(err, context.Canceled) {
				slog.Warn("a dialog's held input was not delivered", "error", err)
			}
		}
	}
}

// deliver preserves held-release order, stopping when a release opens another dialog.
func (d *DialogInput) deliver(
	ctx context.Context,
	releases []InputRelease,
	opening <-chan dialogOpening,
) ([]InputRelease, error) {
	for len(releases) > 0 && !d.IsOpen() {
		release := releases[0]
		releases = releases[1:]
		bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
		sent := make(chan error, 1)
		go func() {
			renderer := protocol.WithExecutor(bounded, d.tab.session)
			var err error
			switch release := release.(type) {
			case *MouseRelease:
				err = release.Event.Do(renderer)
			case *KeyRelease:
				err = release.Event.Do(renderer)
			}
			sent <- err
		}()
		select {
		case event := <-opening:
			cancel()
			<-sent
			if err := d.opened(event); err != nil {
				return releases, err
			}
		case err := <-sent:
			cancel()
			if err != nil {
				return releases, err
			}
		case <-bounded.Done():
			cancel()
			<-sent
			return releases, bounded.Err()
		}
	}
	return releases, nil
}

func (d *DialogInput) answer(ctx context.Context, request dialogRequest) error {
	var err error
	if request.dialog == nil || d.Open() != request.dialog {
		err = &cdp.BrowserError{Kind: cdp.KindDialogNotFound}
	} else {
		bounded, cancel := context.WithTimeout(ctx, 30*time.Second)
		command := page.HandleJavaScriptDialog(request.accept)
		if request.text != nil {
			command = command.WithPromptText(*request.text)
		}
		err = command.Do(protocol.WithExecutor(bounded, d.tab.session))
		cancel()
		if err == nil {
			d.publish(nil)
		}
	}
	return err
}

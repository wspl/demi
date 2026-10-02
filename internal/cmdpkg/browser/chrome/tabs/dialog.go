package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/chromedp/cdproto/input"
	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// InputRelease is a key or mouse release held back while a dialog blocks input.
//
//sumtype:decl
type InputRelease interface{ inputRelease() }

// MouseRelease carries a native mouse button release.
type MouseRelease struct {
	Event *input.DispatchMouseEventParams
}

func (*MouseRelease) inputRelease() {}

// KeyRelease carries a native key release.
type KeyRelease struct{ Event *input.DispatchKeyEventParams }

func (*KeyRelease) inputRelease() {}

// DialogInput is access to the tab-owned dialog and deferred-release worker.
// The tab starts, cancels and joins this worker; consumers do not close it.
type DialogInput struct{}

// Open returns the immutable open dialog, or nil. Preserve its pointer for Answer.
func (d *DialogInput) Open() *page.EventJavascriptDialogOpening { panic("not written: k-chrome-tabs") }

// IsOpen reports whether a dialog currently blocks native input.
func (d *DialogInput) IsOpen() bool { panic("not written: k-chrome-tabs") }

// Watch returns the current dialog and notification from one publication.
func (d *DialogInput) Watch() (*page.EventJavascriptDialogOpening, <-chan struct{}) {
	panic("not written: k-chrome-tabs")
}

// Defer hands releases to the tab owner in order. A full queue waits with ctx;
// after successful admission, the owner retains them beyond caller cancellation.
func (d *DialogInput) Defer(ctx context.Context, releases []InputRelease) error {
	panic("not written: k-chrome-tabs")
}

// Answer answers this exact observed dialog unless someone already answered it,
// then delivers deferred releases. A later dialog is never cleared by this answer.
func (d *DialogInput) Answer(ctx context.Context, dialog *page.EventJavascriptDialogOpening, accept bool, text *string) error {
	panic("not written: k-chrome-tabs")
}

// DialogType converts Chrome's dialog type to the browser result and live-view type.
func DialogType(kind page.DialogType) browserop.DialogType { panic("not written: k-chrome-tabs") }

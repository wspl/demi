package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Capabilities reports what the page command families offer.
func Capabilities(ctx context.Context, tab *tabs.Tab) ([]browserop.Capability, error) {
	panic("not written: k-chrome-page")
}

// ClipboardUnisolated returns why clipboard isolation is unverified, or nil where verified.
func ClipboardUnisolated() *string {
	panic("not written: k-chrome-page")
}

// ClipboardCapability publishes the pinned headless clipboard isolation policy for this document.
func ClipboardCapability(ctx context.Context, tab *tabs.Tab) (browserop.Capability, error) {
	panic("not written: k-chrome-page")
}

// GrantClipboard lets pages use the browser's own clipboard.
func GrantClipboard(ctx context.Context, browser cdp.Executor) error {
	panic("not written: k-chrome-page")
}

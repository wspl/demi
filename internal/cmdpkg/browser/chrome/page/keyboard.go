package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"github.com/chromedp/cdproto/input"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// ViewerKey returns the key event the Host receives for a viewer's key.
func ViewerKey(key browserop.LiveViewerMessageKey, macViewer bool) *input.DispatchKeyEventParams {
	panic("not written: k-chrome-page")
}

// ClickModifiers maps Command-click from a Mac viewer to Control-click on Linux or Windows.
func ClickModifiers(modifiers input.Modifier, macViewer bool) input.Modifier {
	panic("not written: k-chrome-page")
}

// PasteShortcut returns the Host's paste shortcut, down then up.
func PasteShortcut() [2]*input.DispatchKeyEventParams {
	panic("not written: k-chrome-page")
}

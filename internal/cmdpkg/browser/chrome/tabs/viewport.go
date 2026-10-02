package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// PhoneWidth is Mobile mode's width in CSS pixels.
const PhoneWidth uint32 = 390

// PhoneHeight is Mobile mode's height in CSS pixels.
const PhoneHeight uint32 = 844

// Viewports holds the current viewport and the last Web viewport for reset.
type Viewports struct {
	Current browserop.BrowserViewport
	Web     browserop.BrowserViewport
}

// Screen describes the virtual screen shared by the browser's windows.
type Screen struct {
	Width  uint32
	Height uint32
	Ratio  float64
}

// Pixels gives the even encoded picture dimensions at the requested scale.
func Pixels(viewport browserop.BrowserViewport, scale float64) (uint32, uint32) {
	panic("not written: k-chrome-tabs")
}

// ScreenRatio clamps to at least one and rounds upward to whole ratios on macOS.
func ScreenRatio(deviceRatio float64) float64 { panic("not written: k-chrome-tabs") }

// RatioFor limits a viewer ratio to the capture encoder's side and pixel bounds.
func RatioFor(deviceRatio float64, width, height uint32) float64 { panic("not written: k-chrome-tabs") }

// Paint flushes layout to the painted view without resizing it.
func Paint(ctx context.Context, executor cdp.Executor) error { panic("not written: k-chrome-tabs") }

// Viewport returns the last painted and published viewport.
func (t *Tab) Viewport() browserop.BrowserViewport { panic("not written: k-chrome-tabs") }

// WebViewport returns the last Web viewport, used by viewport reset.
func (t *Tab) WebViewport() browserop.BrowserViewport { panic("not written: k-chrome-tabs") }

// Viewports returns an immutable viewport publication and its change notification.
func (t *Tab) Viewports() (Viewports, <-chan struct{}) { panic("not written: k-chrome-tabs") }

// SetViewport sizes the native window before the emulated page, updates Mobile
// emulation and publishes only after painting. Reload policy belongs to the caller.
func (t *Tab) SetViewport(ctx context.Context, viewport browserop.BrowserViewport) error {
	panic("not written: k-chrome-tabs")
}

// UpdateScreen changes the browser's virtual screen while preserving tab viewports.
func (t *Tab) UpdateScreen(ctx context.Context, screen Screen) error {
	panic("not written: k-chrome-tabs")
}

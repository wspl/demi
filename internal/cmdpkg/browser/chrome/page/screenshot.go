package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Screenshot returns a PNG of what the tab shows, taken holding its operation lock.
func Screenshot(ctx context.Context, tab *tabs.Tab, timeout time.Duration) ([]byte, error) {
	panic("not written: k-chrome-page")
}

// Capture returns a PNG of the viewport, whole page, or clip, holding the operation lock.
func Capture(ctx context.Context, tab *tabs.Tab, fullPage bool, clip *string, timeout time.Duration) ([]byte, error) {
	panic("not written: k-chrome-page")
}

// ScreenshotBytes captures CSS pixels without changing the viewport.
// The caller holds the tab checkout and supplies the operation context.
func ScreenshotBytes(ctx context.Context, tab *tabs.Tab, fullPage bool, clip *string) ([]byte, error) {
	panic("not written: k-chrome-page")
}

// AnnotateProbe draws the probe markers onto PNG screenshot bytes.
func AnnotateProbe(data []byte, result browserop.ProbeResult) ([]byte, error) {
	panic("not written: k-chrome-page")
}

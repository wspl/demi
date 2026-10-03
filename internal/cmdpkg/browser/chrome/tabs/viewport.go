package tabs

import (
	"context"
	"errors"
	"fmt"
	"math"
	"runtime"
	"strings"

	"github.com/chromedp/cdproto/browser"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/emulation"
	"github.com/chromedp/cdproto/page"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// PhoneWidth is Mobile mode's width in CSS pixels.
const PhoneWidth uint32 = 390

// PhoneHeight is Mobile mode's height in CSS pixels.
const PhoneHeight uint32 = 844

// Viewports holds the current viewport and the last Web viewport for reset.
type Viewports struct {
	// Current holds the applied viewport.
	Current browserop.BrowserViewport
	// Web holds the panel's web viewport.
	Web browserop.BrowserViewport
}

// Screen describes the virtual screen shared by the browser's windows.
type Screen struct {
	// Width holds the screen width in CSS pixels.
	Width uint32
	// Height holds the screen height in CSS pixels.
	Height uint32
	// Ratio holds the screen device pixel ratio.
	Ratio float64
}

// Pixels gives the even encoded picture dimensions at the requested scale.
func Pixels(viewport browserop.BrowserViewport, scale float64) (uint32, uint32) {
	return uint32(
			math.Ceil(float64(viewport.Width)*viewport.DevicePixelRatio*scale/2),
		) * 2, uint32(
			math.Ceil(float64(viewport.Height)*viewport.DevicePixelRatio*scale/2),
		) * 2
}

// ScreenRatio clamps to at least one and rounds upward to whole ratios on macOS.
func ScreenRatio(deviceRatio float64) float64 {
	ratio := math.Max(deviceRatio, 1)
	if runtime.GOOS == "darwin" {
		return math.Ceil(ratio)
	}
	return ratio
}

// RatioFor limits a viewer ratio to the capture encoder's side and pixel bounds.
func RatioFor(deviceRatio float64, width, height uint32) float64 {
	w, h := float64(width), float64(height)
	limit := math.Min(4096/math.Max(w, h), math.Sqrt(9437184/(w*h)))
	if runtime.GOOS == "darwin" {
		limit = math.Floor(limit)
	}
	return math.Max(1, math.Min(ScreenRatio(deviceRatio), limit))
}

// Paint flushes layout to the painted view without resizing it.
func Paint(ctx context.Context, executor cdp.Executor) error {
	_, err := page.CaptureScreenshot().
		WithFormat(page.CaptureScreenshotFormatJpeg).
		WithQuality(1).
		WithFromSurface(false).
		WithCaptureBeyondViewport(false).
		Do(protocol.WithExecutor(ctx, executor))
	var chrome *cdp.ProtocolError
	// This precise vendor rejection is emitted after the ForceRedraw callback.
	if errors.As(err, &chrome) && chrome.Code == -32000 && chrome.Message == "Unable to capture screenshot" {
		return nil
	}
	return err
}

// Viewport returns the last painted and published viewport.
func (t *Tab) Viewport() browserop.BrowserViewport {
	view, _ := t.Viewports()
	return view.Current
}

// WebViewport returns the last Web viewport, used by viewport reset.
func (t *Tab) WebViewport() browserop.BrowserViewport {
	view, _ := t.Viewports()
	return view.Web
}

// Viewports returns an immutable viewport publication and its change notification.
func (t *Tab) Viewports() (Viewports, <-chan struct{}) {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.viewports, t.viewportChanged
}

// SetViewport sizes the native window before the emulated page, updates Mobile
// emulation and publishes only after painting. Reload policy belongs to the caller.
func (t *Tab) SetViewport(ctx context.Context, viewport browserop.BrowserViewport) error {
	renderer := protocol.WithExecutor(ctx, t.Page())
	mobile := viewport.Mode == browserop.ViewportModeMobile
	if mobile != (t.Viewport().Mode == browserop.ViewportModeMobile) {
		if err := setMobileEmulation(renderer, mobile); err != nil {
			return err
		}
	}
	browserCtx := protocol.WithExecutor(ctx, t.Browser())
	window, _, err := browser.GetWindowForTarget().WithTargetID(t.TargetID()).Do(browserCtx)
	if err != nil {
		return err
	}
	if err := browser.SetWindowBounds(
		window,
		&browser.Bounds{
			Width: int64(
				viewport.Width,
			),
			Height: int64(
				viewport.Height,
			) + 87,
		},
	).
		Do(browserCtx); err != nil {
		return err
	}
	if err := emulation.SetDeviceMetricsOverride(
		int64(
			viewport.Width,
		),
		int64(
			viewport.Height,
		),
		viewport.DevicePixelRatio,
		mobile,
	).
		Do(renderer); err != nil {
		return err
	}
	if err := Paint(ctx, t.Page()); err != nil {
		return err
	}
	t.mu.Lock()
	t.viewports.Current = viewport
	if viewport.Mode == browserop.ViewportModeWeb {
		t.viewports.Web = viewport
	}
	old := t.viewportChanged
	t.viewportChanged = make(chan struct{})
	t.mu.Unlock()
	close(old)
	t.environment.markChanged()
	return nil
}

// UpdateScreen changes the browser's virtual screen while preserving tab viewports.
func (t *Tab) UpdateScreen(ctx context.Context, screen Screen) error {
	renderer := protocol.WithExecutor(ctx, t.Page())
	screens, err := emulation.GetScreenInfos().Do(renderer)
	if err != nil {
		return err
	}
	if len(screens) == 0 {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "the browser has no screen"}
	}
	params := emulation.UpdateScreen(screens[0].ID).
		WithWidth(int64(math.Round(float64(screen.Width) * screen.Ratio))).
		WithHeight(int64(math.Round(float64(screen.Height) * screen.Ratio))).
		WithDevicePixelRatio(screen.Ratio)
	// The method's result is not needed; no vendor fields are read here.
	return t.Page().Execute(ctx, emulation.CommandUpdateScreen, params, nil)
}

// unwatchedViewport is the initial viewport of a tab no viewer has resized.
func unwatchedViewport() browserop.BrowserViewport {
	return browserop.BrowserViewport{Width: 1280, Height: 720, DevicePixelRatio: 1, Mode: browserop.ViewportModeWeb}
}

func setMobileEmulation(ctx context.Context, mobile bool) error {
	agent := emulation.SetUserAgentOverride("")
	if mobile {
		version, err := PinnedVersion()
		if err != nil {
			return err
		}
		major, _, _ := strings.Cut(version, ".")
		agent = emulation.SetUserAgentOverride(fmt.Sprintf(
			"Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/%s.0.0.0 "+
				"Mobile Safari/537.36", major)).
			WithPlatform("Linux armv8l").
			WithUserAgentMetadata(&emulation.UserAgentMetadata{
				Platform:        "Android",
				PlatformVersion: "14.0.0",
				Architecture:    "",
				Model:           "Pixel 7",
				Mobile:          true,
			})
	}
	if err := agent.Do(ctx); err != nil {
		return err
	}
	touch := emulation.SetTouchEmulationEnabled(mobile)
	if mobile {
		touch = touch.WithMaxTouchPoints(5)
	}
	if err := touch.Do(ctx); err != nil {
		return err
	}
	return nil
}
